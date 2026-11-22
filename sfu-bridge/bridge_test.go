package main

import (
	"testing"

	"github.com/pion/rtp"
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
	t.Setenv("BACKEND_INTERNAL_URL", "http://backend:8000/")
	t.Setenv("SFU_BRIDGE_TOKEN", "short")
	if _, err := configFromEnv(); err == nil {
		t.Fatal("short token must fail")
	}
	t.Setenv("SFU_BRIDGE_TOKEN", "0123456789abcdef0123456789abcdef")
	cfg, err := configFromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.backendURL != "http://backend:8000" || cfg.healthAddr != ":8080" {
		t.Fatalf("unexpected config: %+v", cfg)
	}
}

func TestRoomKeySeparatesEpochs(t *testing.T) {
	if roomKey(roomState{ChannelID: "c", Epoch: 1}) == roomKey(roomState{ChannelID: "c", Epoch: 2}) {
		t.Fatal("epochs must not share a bridge")
	}
}
