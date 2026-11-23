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
	b.unpark("b", nil)
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.parked) != 0 {
		t.Fatal("unpark must drop the track")
	}
}

// fakeBackend は cf/... の要求を記録し、決まった応答を返す。
type fakeBackend struct {
	mu       sync.Mutex
	paths    []string
	closed   [][]string
	pullResp tracksResponse
}

func (f *fakeBackend) handler(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.paths = append(f.paths, r.URL.Path)
	switch r.URL.Path {
	case "/internal/sfu-bridge/cf/pull":
		_ = json.NewEncoder(w).Encode(f.pullResp)
	case "/internal/sfu-bridge/cf/close":
		var body struct {
			Mids []string `json:"mids"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		f.closed = append(f.closed, body.Mids)
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
	b.mu.Lock()
	b.pulled["x:direct"] = &pulledTrack{speaker: "x"}
	b.pulled["y:direct"] = &pulledTrack{speaker: "y"}
	b.mu.Unlock()
	b.pull([]string{"x:direct", "y:direct"})
	b.mu.Lock()
	defer b.mu.Unlock()
	if p := b.pulled["x:direct"]; p == nil || p.mid != "0" || b.midSpeaker["0"] != "x" {
		t.Fatalf("the successful pull must be kept: %+v", p)
	}
	if _, ok := b.pulled["y:direct"]; ok {
		t.Fatal("the failed pull must be released so that it is retried")
	}
}

func TestPullResponseAfterRemovalIsClosed(t *testing.T) {
	f := &fakeBackend{pullResp: tracksResponse{Tracks: []trackResult{{Mid: "3", TrackName: "x"}}}}
	b := readyBridge(t, f)
	// 応答を待つあいだに、update が予約を消した。
	b.pull([]string{"x:direct"})
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.closed) != 1 || len(f.closed[0]) != 1 || f.closed[0][0] != "3" {
		t.Fatalf("the orphaned mid must be closed: %v", f.closed)
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
