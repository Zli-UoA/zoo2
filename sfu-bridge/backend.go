package main

import (
	"bytes"
	"context"
	"encoding/json"
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
		return fmt.Errorf("%s %s: status %d", method, path, res.StatusCode)
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(data, out)
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

func (a *backendAPI) cfPublish(ctx context.Context, channelID string, bridgeEpoch int64, cfSession, speaker string, offer sessionDescription, mid string) (tracksResponse, error) {
	var out tracksResponse
	err := a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/publish", map[string]any{
		"channel_id":         channelID,
		"bridge_epoch":       bridgeEpoch,
		"cf_session_id":      cfSession,
		"speaker_session_id": speaker,
		"offer":              offer,
		"mid":                mid,
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

func (a *backendAPI) cfClose(ctx context.Context, cfSession string, mids []string) error {
	return a.do(ctx, http.MethodPost, "/internal/sfu-bridge/cf/close", map[string]any{
		"cf_session_id": cfSession,
		"mids":          mids,
	}, nil)
}
