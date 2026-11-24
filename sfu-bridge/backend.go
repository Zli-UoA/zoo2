package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"time"
)

// backendAPI は backend の /internal/sfu-bridge/... を呼ぶ。
type backendAPI struct {
	base  string
	token string
	http  *http.Client
}

func newBackendAPI(base, token string) *backendAPI {
	return &backendAPI{base: base, token: token, http: &http.Client{
		Timeout: 40 * time.Second,
		// リダイレクトには従わない。従うと、Authorization ヘッダーが別の宛先（http を含む）へ送られうる。
		// 3xx は do でエラーにする。
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}}
}

type sessionDescription struct {
	Type string `json:"type"`
	SDP  string `json:"sdp"`
}

type trackResult struct {
	Mid       string `json:"mid"`
	TrackName string `json:"trackName"`
	ErrorCode string `json:"errorCode"`
}

type tracksResponse struct {
	SessionDescription             *sessionDescription `json:"sessionDescription"`
	RequiresImmediateRenegotiation bool                `json:"requiresImmediateRenegotiation"`
	Tracks                         []trackResult       `json:"tracks"`
}

type trackInfo struct {
	ID        string `json:"id"`
	SessionID string `json:"session_id"`
	Source    string `json:"source"`
	// Rev は公開の世代（Cloudflare のセッションと track 名のハッシュ）。同じ id でも変われば別の音声。
	// 古い backend は返さないので、無ければ ""。
	Rev string `json:"rev"`
}

type roomState struct {
	ChannelID        string      `json:"channel_id"`
	Epoch            int64       `json:"epoch"`
	LivekitSessions  []string    `json:"livekit_sessions"`
	CloudflareTracks []trackInfo `json:"cloudflare_tracks"`
	LivekitURL       string      `json:"livekit_url"`
	LivekitRoom      string      `json:"livekit_room"`
	LivekitToken     string      `json:"livekit_token"`
}

type roomsResponse struct {
	Revision string      `json:"revision"`
	Rooms    []roomState `json:"rooms"`
}

func (a *backendAPI) do(ctx context.Context, method, path string, body, out any) error {
	var reader io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		reader = bytes.NewReader(b)
	}
	req, err := http.NewRequestWithContext(ctx, method, a.base+path, reader)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+a.token)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	res, err := a.http.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	data, err := io.ReadAll(io.LimitReader(res.Body, 1<<20))
	if err != nil {
		return err
	}
	if res.StatusCode >= 300 {
		return &statusError{method: method, path: path, status: res.StatusCode}
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(data, out)
}

// statusError は、backend が 2xx 以外を返した。
type statusError struct {
	method, path string
	status       int
}

func (e *statusError) Error() string {
	return fmt.Sprintf("%s %s: status %d", e.method, e.path, e.status)
}

// rejectedBeforeApply は、backend が要求を Cloudflare に送る前に断った（4xx）か。
// 通信の失敗・時間切れ・5xx（Cloudflare の失敗の中継を含む）・応答の読み取りの失敗では、Cloudflare で
// 処理されたかどうか分からないので false。
func rejectedBeforeApply(err error) bool {
	var se *statusError
	return errors.As(err, &se) && se.status >= 400 && se.status < 500
}

func (a *backendAPI) register(ctx context.Context) (int64, error) {
	var out struct {
		BridgeEpoch int64 `json:"bridge_epoch"`
	}
	err := a.do(ctx, http.MethodPost, "/internal/sfu-bridge/register", nil, &out)
	return out.BridgeEpoch, err
}

// rooms は、前回の revision から変化があるか、25 秒たつまで待ってから返る。
func (a *backendAPI) rooms(ctx context.Context, bridgeEpoch int64, since string) (roomsResponse, error) {
	q := url.Values{}
	q.Set("bridge_epoch", strconv.FormatInt(bridgeEpoch, 10))
	if since != "" {
		q.Set("since", since)
	}
	var out roomsResponse
	err := a.do(ctx, http.MethodGet, "/internal/sfu-bridge/rooms?"+q.Encode(), nil, &out)
	return out, err
}

func (a *backendAPI) cfSession(ctx context.Context, channelID string) (string, error) {
	var out struct {
		CfSessionID string `json:"cf_session_id"`
	}
	err := a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/session", map[string]any{"channel_id": channelID}, &out)
	return out.CfSessionID, err
}

func (a *backendAPI) cfPublish(ctx context.Context, channelID string, bridgeEpoch int64, cfSession, speaker string, offer sessionDescription, mid, publishID string) (tracksResponse, error) {
	var out tracksResponse
	err := a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/publish", map[string]any{
		"channel_id":         channelID,
		"bridge_epoch":       bridgeEpoch,
		"cf_session_id":      cfSession,
		"speaker_session_id": speaker,
		"offer":              offer,
		"mid":                mid,
		"publish_id":         publishID,
	}, &out)
	return out, err
}

func (a *backendAPI) cfPull(ctx context.Context, channelID, cfSession string, trackIDs []string) (tracksResponse, error) {
	var out tracksResponse
	err := a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/pull", map[string]any{
		"channel_id":    channelID,
		"cf_session_id": cfSession,
		"track_ids":     trackIDs,
	}, &out)
	return out, err
}

func (a *backendAPI) cfAnswer(ctx context.Context, cfSession string, answer sessionDescription) error {
	return a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/answer", map[string]any{
		"cf_session_id": cfSession,
		"answer":        answer,
	}, nil)
}

// cfClose は mid を閉じる。pushes は、閉じる mid のうち bridge の push（話者と push の ID）。backend は閉じる前に、
// その push の行を一覧から外す（Cloudflare に問い合わせずに済むよう、ここで渡す）。
func (a *backendAPI) cfClose(ctx context.Context, channelID, cfSession string, mids []string, pushes []closedPush) error {
	if pushes == nil {
		pushes = []closedPush{}
	}
	return a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/close", map[string]any{
		"channel_id":    channelID,
		"cf_session_id": cfSession,
		"mids":          mids,
		"pushes":        pushes,
	}, nil)
}
