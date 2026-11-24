package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/pion/rtp"
	"github.com/pion/webrtc/v4"
)

func TestSpeakerOfTrackName(t *testing.T) {
	if got := speakerOfTrackName("abc-bridge"); got != "abc" {
		t.Fatalf("got %q", got)
	}
	if got := speakerOfTrackName("abc"); got != "abc" {
		t.Fatalf("got %q", got)
	}
}

func TestStripExtensions(t *testing.T) {
	pkt := &rtp.Packet{Header: rtp.Header{Extension: true, ExtensionProfile: 0xBEDE}}
	if err := pkt.Header.SetExtension(1, []byte{0x10}); err != nil {
		t.Fatal(err)
	}
	stripExtensions(pkt)
	if pkt.Header.Extension || len(pkt.Header.Extensions) != 0 || pkt.Header.ExtensionProfile != 0 {
		t.Fatalf("extensions remain: %+v", pkt.Header)
	}
}

func TestConfigFromEnv(t *testing.T) {
	t.Setenv("BACKEND_INTERNAL_URL", "")
	t.Setenv("SFU_BRIDGE_TOKEN", "")
	if _, err := configFromEnv(); err == nil {
		t.Fatal("empty config must fail")
	}
	t.Setenv("BACKEND_INTERNAL_URL", "http://backend.zoo2.svc:8000/")
	t.Setenv("SFU_BRIDGE_TOKEN", "short")
	if _, err := configFromEnv(); err == nil {
		t.Fatal("short token must fail")
	}
	t.Setenv("SFU_BRIDGE_TOKEN", "0123456789abcdef0123456789abcdef")
	cfg, err := configFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.backendURL != "http://backend.zoo2.svc:8000" || cfg.healthAddr != ":8080" {
		t.Fatalf("unexpected config: %+v", cfg)
	}
}

func TestRoomKeySeparatesEpochs(t *testing.T) {
	if roomKey(roomState{ChannelID: "c", Epoch: 1}) == roomKey(roomState{ChannelID: "c", Epoch: 2}) {
		t.Fatal("epochs must not share a bridge")
	}
}

func TestCheckBackendURL(t *testing.T) {
	ok := []string{
		"https://backend.example.com",
		"http://backend.zoo2.svc:8000",
		"http://backend.zoo2.svc.cluster.local:8000",
		"http://localhost:8000",
		"http://127.0.0.1:8000",
		"http://[::1]:8000",
	}
	for _, u := range ok {
		if err := checkBackendURL(u); err != nil {
			t.Errorf("%s: %v", u, err)
		}
	}
	ng := []string{
		"http://backend:8000",
		"http://backend.example.com",
		"http://svc.example.com",
		"http://10.0.0.1:8000",
		"ftp://backend.zoo2.svc",
		"backend.zoo2.svc:8000",
		"https://",
		`http://evil.example\@backend.zoo2.svc:8000/`,
		`https://backend.zoo2.svc\@evil.example/`,
		"http://user@backend.zoo2.svc:8000",
	}
	for _, u := range ng {
		if err := checkBackendURL(u); err == nil {
			t.Errorf("%s must be rejected", u)
		}
	}
}

func TestBackendDoesNotFollowRedirects(t *testing.T) {
	var followed sync.Map
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/elsewhere" {
			followed.Store("hit", r.Header.Get("Authorization"))
			return
		}
		http.Redirect(w, r, "/elsewhere", http.StatusFound)
	}))
	defer srv.Close()
	api := newBackendAPI(srv.URL, "0123456789abcdef0123456789abcdef")
	if _, err := api.register(context.Background()); err == nil {
		t.Fatal("a redirect must be an error")
	}
	if _, ok := followed.Load("hit"); ok {
		t.Fatal("the redirect was followed")
	}
}

func TestOwnClosesWhatConnectFinishesAfterClose(t *testing.T) {
	b := newRoomBridge(context.Background(), nil, 1, roomState{ChannelID: "c"})
	set, closed := false, false
	if !b.own(func() { set = true }, func() { closed = true }) || !set || closed {
		t.Fatal("before close, own must keep it")
	}
	b.close()
	set, closed = false, false
	if b.own(func() { set = true }, func() { closed = true }) || set || !closed {
		t.Fatal("after close, own must close it")
	}
}

func TestUpdateWhileConnectingAllowsSpeakers(t *testing.T) {
	b := newRoomBridge(context.Background(), nil, 1, roomState{ChannelID: "c", LivekitSessions: []string{"a"}})
	defer b.close()
	// 接続中に届いた、一覧にない話者のトラックは取っておく（転送しない）。
	b.markSubscribed(nil)
	b.pushToCloudflare("b", nil)
	b.mu.Lock()
	allowedA, parked := b.lkSpeakers["a"], len(b.parked["b"])
	b.mu.Unlock()
	if !allowedA || parked != 1 {
		t.Fatalf("allowedA=%v parked=%d", allowedA, parked)
	}
	b.update(roomState{ChannelID: "c", LivekitSessions: []string{"a", "b"}})
	b.mu.Lock()
	allowedB, latest := b.lkSpeakers["b"], len(b.latest.LivekitSessions)
	b.mu.Unlock()
	if !allowedB || latest != 2 {
		t.Fatal("update while connecting must record the latest list")
	}
	b.unsubscribe("b", nil)
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.parked) != 0 || len(b.subscribed) != 0 {
		t.Fatal("unsubscribe must drop the track")
	}
}

// fakeBackend は cf/... の要求を記録し、決まった応答を返す。
type fakeBackend struct {
	mu         sync.Mutex
	paths      []string
	closed     [][]string
	closeChans []string
	closeSpk   [][]string
	pullResp   tracksResponse
	pullStatus int
	// closeFails は、失敗させる close の残りの回数。
	closeFails int
}

func (f *fakeBackend) handler(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.paths = append(f.paths, r.URL.Path)
	switch r.URL.Path {
	case "/internal/sfu-bridge/cf/pull":
		if f.pullStatus != 0 {
			w.WriteHeader(f.pullStatus)
			return
		}
		_ = json.NewEncoder(w).Encode(f.pullResp)
	case "/internal/sfu-bridge/cf/close":
		if f.closeFails > 0 {
			f.closeFails--
			w.WriteHeader(http.StatusBadGateway)
			return
		}
		var body struct {
			ChannelID string   `json:"channel_id"`
			Mids      []string `json:"mids"`
			Speakers  []string `json:"speakers"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		f.closed = append(f.closed, body.Mids)
		f.closeSpk = append(f.closeSpk, body.Speakers)
		f.closeChans = append(f.closeChans, body.ChannelID)
	default:
		w.WriteHeader(http.StatusInternalServerError)
	}
}

func readyBridge(t *testing.T, f *fakeBackend) *roomBridge {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(f.handler))
	t.Cleanup(srv.Close)
	b := newRoomBridge(context.Background(), newBackendAPI(srv.URL, "0123456789abcdef0123456789abcdef"), 1, roomState{ChannelID: "c"})
	pc, err := webrtc.NewPeerConnection(webrtc.Configuration{})
	if err != nil {
		t.Fatal(err)
	}
	b.pc = pc
	b.cfSession = "s"
	close(b.ready)
	t.Cleanup(b.close)
	return b
}

func TestPullErrorCodeReleasesReservation(t *testing.T) {
	f := &fakeBackend{pullResp: tracksResponse{Tracks: []trackResult{
		{Mid: "0", TrackName: "x"},
		{TrackName: "y", ErrorCode: "not_found"},
	}}}
	b := readyBridge(t, f)
	x, y := reserve(b, "x:direct", "", "x"), reserve(b, "y:direct", "", "y")
	b.pull([]*pulledTrack{x, y})
	b.mu.Lock()
	defer b.mu.Unlock()
	if p := b.pulled[pullKey("x:direct", "")]; p == nil || p.mid != "0" || b.midSpeaker["0"] != "x" {
		t.Fatalf("the successful pull must be kept: %+v", p)
	}
	if _, ok := b.pulled[pullKey("y:direct", "")]; ok {
		t.Fatal("the failed pull must be released so that it is retried")
	}
}

func TestPullResponseAfterRemovalIsClosed(t *testing.T) {
	f := &fakeBackend{pullResp: tracksResponse{Tracks: []trackResult{{Mid: "3", TrackName: "x"}}}}
	b := readyBridge(t, f)
	// 応答を待つあいだに、update が予約を消した。
	b.pull([]*pulledTrack{{id: "x:direct", speaker: "x"}})
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || len(f.closed[0]) != 1 || f.closed[0][0] != "3" {
		t.Fatalf("the orphaned mid must be closed: %v", f.closed)
	}
	// pull の mid を閉じるだけなので、話者は渡さない（一覧から外さない）。
	if len(f.closeSpk[0]) != 0 {
		t.Fatalf("closing pulled mids must not pass speakers: %v", f.closeSpk)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if _, ok := b.midSpeaker["3"]; ok {
		t.Fatal("the orphaned mid must not be forwarded")
	}
}

func TestPublishFailureFailsBridge(t *testing.T) {
	f := &fakeBackend{}
	b := readyBridge(t, f)
	b.mu.Lock()
	b.parked["a"] = []*webrtc.TrackRemote{nil}
	b.subscribed[nil] = true
	b.mu.Unlock()
	// 一覧に載ると、取っておいたトラックを push する。cf/publish が 500 を返すので、中継は失敗になる。
	b.update(roomState{ChannelID: "c", LivekitSessions: []string{"a"}})
	deadline := time.Now().Add(10 * time.Second)
	for !b.failed() {
		if time.Now().After(deadline) {
			t.Fatal("a publish failure must mark the bridge failed")
		}
		time.Sleep(10 * time.Millisecond)
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.parked) != 0 {
		t.Fatal("the parked track must be taken")
	}
}

// reserve は sync と同じように予約を入れる。
func reserve(b *roomBridge, id, rev, speaker string) *pulledTrack {
	p := &pulledTrack{id: id, rev: rev, speaker: speaker}
	b.mu.Lock()
	b.pulled[pullKey(id, rev)] = p
	b.mu.Unlock()
	return p
}

// reRegister は、pull の応答を待つあいだに同じ id が一覧から外れて載り直したことにする（sync が新しい予約を入れる）。
func reRegister(b *roomBridge, id, speaker string) *pulledTrack {
	b.mu.Lock()
	delete(b.pulled, pullKey(id, ""))
	b.mu.Unlock()
	return reserve(b, id, "", speaker)
}

func TestOldPullSuccessDoesNotFillReRegisteredReservation(t *testing.T) {
	f := &fakeBackend{pullResp: tracksResponse{Tracks: []trackResult{{Mid: "3", TrackName: "x"}}}}
	b := readyBridge(t, f)
	old := reserve(b, "x:direct", "", "x")
	fresh := reRegister(b, "x:direct", "x")
	b.pull([]*pulledTrack{old})
	b.mu.Lock()
	cur, fwd := b.pulled[pullKey("x:direct", "")], b.midSpeaker["3"]
	b.mu.Unlock()
	if cur != fresh || fresh.mid != "" || old.mid != "" || fwd != "" {
		t.Fatalf("the old response must not fill the new reservation: cur=%+v fwd=%q", cur, fwd)
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || len(f.closed[0]) != 1 || f.closed[0][0] != "3" {
		t.Fatalf("the old mid must be closed as an orphan: %v", f.closed)
	}
}

func TestOldPullFailureKeepsReRegisteredReservation(t *testing.T) {
	cases := map[string]*fakeBackend{
		"error_code": {pullResp: tracksResponse{Tracks: []trackResult{{TrackName: "x", ErrorCode: "not_found"}}}},
		"http_error": {pullStatus: http.StatusNotFound},
	}
	for name, f := range cases {
		t.Run(name, func(t *testing.T) {
			b := readyBridge(t, f)
			old := reserve(b, "x:direct", "", "x")
			fresh := reRegister(b, "x:direct", "x")
			b.pull([]*pulledTrack{old})
			b.mu.Lock()
			defer b.mu.Unlock()
			if cur := b.pulled[pullKey("x:direct", "")]; cur != fresh {
				t.Fatalf("the old failure must not release the new reservation: %+v", cur)
			}
		})
	}
}

func TestRevChangeRepulls(t *testing.T) {
	f := &fakeBackend{pullResp: tracksResponse{Tracks: []trackResult{{Mid: "6", TrackName: "x"}}}}
	b := readyBridge(t, f)
	old := reserve(b, "x:direct", "r1", "x")
	b.mu.Lock()
	old.mid = "5"
	b.midSpeaker["5"] = "x"
	b.mu.Unlock()
	// 同じ id で rev が変わった（話者が Cloudflare に publish し直した）。
	b.update(roomState{ChannelID: "c", CloudflareTracks: []trackInfo{{ID: "x:direct", SessionID: "x", Source: "direct", Rev: "r2"}}})
	deadline := time.Now().Add(10 * time.Second)
	for {
		b.mu.Lock()
		p := b.pulled[pullKey("x:direct", "r2")]
		done := p != nil && p.mid == "6"
		_, oldKept := b.pulled[pullKey("x:direct", "r1")]
		_, oldFwd := b.midSpeaker["5"]
		b.mu.Unlock()
		if done {
			if oldKept || oldFwd {
				t.Fatal("the old subscription must be dropped")
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("the new rev must be pulled")
		}
		time.Sleep(10 * time.Millisecond)
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || len(f.closed[0]) != 1 || f.closed[0][0] != "5" {
		t.Fatalf("the old mid must be closed: %v", f.closed)
	}
}

func TestMissingRevIsEmpty(t *testing.T) {
	var r roomState
	if err := json.Unmarshal([]byte(`{"cloudflare_tracks":[{"id":"x:direct","session_id":"x","source":"direct"}]}`), &r); err != nil {
		t.Fatal(err)
	}
	if len(r.CloudflareTracks) != 1 || r.CloudflareTracks[0].Rev != "" {
		t.Fatalf("unexpected: %+v", r.CloudflareTracks)
	}
}

func TestUnsubscribedBeforePushIsNotRelayed(t *testing.T) {
	f := &fakeBackend{}
	b := readyBridge(t, f)
	// OnTrackSubscribed が goroutine を起こしたが、動く前に OnTrackUnsubscribed が来た。
	b.markSubscribed(nil)
	b.unsubscribe("b", nil)
	b.pushToCloudflare("b", nil)
	b.mu.Lock()
	parked := len(b.parked)
	b.mu.Unlock()
	if parked != 0 {
		t.Fatal("an unsubscribed track must not be parked")
	}
	// 話者が一覧に載っても push しない（cf/publish を呼ばない）。
	b.update(roomState{ChannelID: "c", LivekitSessions: []string{"b"}})
	b.pushToCloudflare("b", nil)
	b.relayToCloudflare("b", nil)
	f.mu.Lock()
	defer f.mu.Unlock()
	for _, p := range f.paths {
		if p == "/internal/sfu-bridge/cf/publish" {
			t.Fatal("an unsubscribed track must not be published")
		}
	}
	if b.failed() {
		t.Fatal("the bridge must not fail")
	}
}

func TestSessionOfIdentity(t *testing.T) {
	sid := "6f1c1a2e-0000-4000-8000-000000000001"
	cases := map[string]string{
		sid + ".0123456789abcdef": sid,
		sid + ".a.b":              sid,
		sid:                       sid,
	}
	for identity, want := range cases {
		if got := sessionOfIdentity(identity); got != want {
			t.Errorf("sessionOfIdentity(%q) = %q, want %q", identity, got, want)
		}
	}
	// 割り当てが変わって identity が変わっても、一覧の同じ session_id として扱う。
	if sessionOfIdentity(sid+".1") != sessionOfIdentity(sid+".2") {
		t.Error("identities of the same session differ")
	}
}

func TestPullFailureWithUnknownOutcomeFailsBridge(t *testing.T) {
	for _, status := range []int{http.StatusBadGateway, http.StatusInternalServerError} {
		f := &fakeBackend{pullStatus: status}
		b := readyBridge(t, f)
		x := reserve(b, "x:direct", "", "x")
		b.pull([]*pulledTrack{x})
		if !b.failed() {
			t.Fatalf("status %d: a pull whose outcome is unknown must rebuild the bridge", status)
		}
	}
}

func TestPullRejectedByBackendIsRetried(t *testing.T) {
	f := &fakeBackend{pullStatus: http.StatusNotFound}
	b := readyBridge(t, f)
	x := reserve(b, "x:direct", "", "x")
	b.pull([]*pulledTrack{x})
	if b.failed() {
		t.Fatal("a pull the backend rejected must not rebuild the bridge")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if _, ok := b.pulled[pullKey("x:direct", "")]; ok {
		t.Fatal("the rejected pull must be released so that it is retried")
	}
}

func TestPullLostResponseFailsBridge(t *testing.T) {
	// 応答を返さずに接続を切る（応答の喪失。時間切れと同じく、Cloudflare が受けたか分からない）。
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, _, err := w.(http.Hijacker).Hijack()
		if err == nil {
			_ = conn.Close()
		}
	}))
	t.Cleanup(srv.Close)
	b := newRoomBridge(context.Background(), newBackendAPI(srv.URL, "0123456789abcdef0123456789abcdef"), 1, roomState{ChannelID: "c"})
	t.Cleanup(b.close)
	b.cfSession = "s"
	close(b.ready)
	x := reserve(b, "x:direct", "", "x")
	b.pull([]*pulledTrack{x})
	if !b.failed() {
		t.Fatal("a pull whose response was lost must rebuild the bridge")
	}
}

func TestUnpushClosesUnderNegotiateWithChannel(t *testing.T) {
	f := &fakeBackend{}
	b := readyBridge(t, f)
	local, err := webrtc.NewTrackLocalStaticRTP(opusCapability, "audio", "a")
	if err != nil {
		t.Fatal(err)
	}
	tr, err := b.pc.AddTransceiverFromTrack(local, webrtc.RTPTransceiverInit{Direction: webrtc.RTPTransceiverDirectionSendonly})
	if err != nil {
		t.Fatal(err)
	}
	b.pushed["a"] = &pushedTrack{mid: "4", transceiver: tr}
	// 交渉中は、閉じるのを待つ。
	b.negotiate.Lock()
	done := make(chan struct{})
	go func() {
		b.unpush("a", "4", tr)
		close(done)
	}()
	select {
	case <-done:
		t.Fatal("unpush must wait for the ongoing negotiation")
	case <-time.After(50 * time.Millisecond):
	}
	b.negotiate.Unlock()
	<-done
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || len(f.closed[0]) != 1 || f.closed[0][0] != "4" || f.closeChans[0] != "c" {
		t.Fatalf("unpush must close the mid with the channel: %v %v", f.closed, f.closeChans)
	}
	// backend が close の前に一覧から外せるよう、push をやめた話者を渡す。
	if len(f.closeSpk[0]) != 1 || f.closeSpk[0][0] != "a" {
		t.Fatalf("unpush must pass the speaker: %v", f.closeSpk)
	}
	if _, ok := b.pushed["a"]; ok {
		t.Fatal("unpush must forget the push")
	}
}

func TestFailedCloseIsRetried(t *testing.T) {
	old := closeRetryInterval
	closeRetryInterval = 10 * time.Millisecond
	t.Cleanup(func() { closeRetryInterval = old })
	f := &fakeBackend{closeFails: 1}
	b := readyBridge(t, f)
	b.negotiate.Lock()
	b.closeMidsLocked([]string{"4"}, []string{"a"})
	b.negotiate.Unlock()
	deadline := time.Now().Add(2 * time.Second)
	for {
		f.mu.Lock()
		n := len(f.closed)
		f.mu.Unlock()
		if n == 1 || time.Now().After(deadline) {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || f.closed[0][0] != "4" || len(f.closeSpk[0]) != 1 || f.closeSpk[0][0] != "a" {
		t.Fatalf("the failed close must be retried with its speaker: %v %v", f.closed, f.closeSpk)
	}
}

func TestRetriedCloseKeepsRepushedSpeaker(t *testing.T) {
	old := closeRetryInterval
	closeRetryInterval = time.Hour
	t.Cleanup(func() { closeRetryInterval = old })
	f := &fakeBackend{closeFails: 1}
	b := readyBridge(t, f)
	b.negotiate.Lock()
	b.closeMidsLocked([]string{"4"}, []string{"a"})
	// やり直す前に、同じ話者を push し直した。
	b.mu.Lock()
	b.pushed["a"] = &pushedTrack{mid: "6"}
	b.mu.Unlock()
	b.retryPendingClosesLocked()
	b.negotiate.Unlock()
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || f.closed[0][0] != "4" || len(f.closeSpk[0]) != 0 {
		t.Fatalf("the retry must close the mid but keep the re-pushed speaker's row: %v %v", f.closed, f.closeSpk)
	}
}

func TestUnpushKeepsRepushedSpeaker(t *testing.T) {
	f := &fakeBackend{}
	b := readyBridge(t, f)
	local, err := webrtc.NewTrackLocalStaticRTP(opusCapability, "audio", "a")
	if err != nil {
		t.Fatal(err)
	}
	tr, err := b.pc.AddTransceiverFromTrack(local, webrtc.RTPTransceiverInit{Direction: webrtc.RTPTransceiverDirectionSendonly})
	if err != nil {
		t.Fatal(err)
	}
	// 古い push（mid 4）を止める前に、同じ話者の新しい push（mid 6）が済んでいる。
	b.pushed["a"] = &pushedTrack{mid: "6"}
	b.unpush("a", "4", tr)
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || f.closed[0][0] != "4" || len(f.closeSpk[0]) != 0 {
		t.Fatalf("closing the old push must not remove the new push's row: %v %v", f.closed, f.closeSpk)
	}
}

func TestCloseRetriesAfterGivingUp(t *testing.T) {
	old := closeRetryInterval
	closeRetryInterval = time.Hour
	t.Cleanup(func() { closeRetryInterval = old })
	f := &fakeBackend{closeFails: closeRetryMaxAttempts + 2}
	b := readyBridge(t, f)
	b.negotiate.Lock()
	b.closeMidsLocked([]string{"4"}, nil)
	for i := 0; i < closeRetryMaxAttempts; i++ {
		b.closeRetry.Stop()
		b.closeRetry = nil
		b.retryPendingClosesLocked()
	}
	// 上限で諦めた。あとの close が一度失敗しても、やり直しを予約し、送り直せば届く。
	if len(b.pendingCloses) != 0 || b.closeRetry != nil {
		t.Fatalf("the first close must be given up: %v", b.pendingCloses)
	}
	b.closeMidsLocked([]string{"5"}, nil)
	if b.closeRetry == nil {
		t.Fatal("a later failed close must be retried")
	}
	b.closeRetry.Stop()
	b.closeRetry = nil
	b.retryPendingClosesLocked()
	b.negotiate.Unlock()
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || f.closed[0][0] != "5" {
		t.Fatalf("the later close must be sent on retry: %v", f.closed)
	}
}
