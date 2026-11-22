package main

import (
	"context"
	"errors"
	"log/slog"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/livekit/protocol/livekit"
	lksdk "github.com/livekit/server-sdk-go/v2"
	"github.com/pion/rtp"
	"github.com/pion/webrtc/v4"
)

const negotiateTimeout = 15 * time.Second

var opusCapability = webrtc.RTPCodecCapability{MimeType: webrtc.MimeTypeOpus, ClockRate: 48000, Channels: 2}

func itoa(v int64) string { return strconv.FormatInt(v, 10) }

// speakerOfTrackName は Cloudflare の track 名から話者の session_id を得る。
func speakerOfTrackName(name string) string { return strings.TrimSuffix(name, "-bridge") }

// stripExtensions は RTP ヘッダ拡張を外す。拡張の ID の割り当ては LiveKit と Cloudflare で違うため。
func stripExtensions(pkt *rtp.Packet) {
	pkt.Header.Extension = false
	pkt.Header.ExtensionProfile = 0
	pkt.Header.Extensions = nil
}

type pulledTrack struct {
	speaker string
	mid     string
}

type pushedTrack struct {
	mid         string
	transceiver *webrtc.RTPTransceiver
}

// roomBridge は 1 つの通話（channel と epoch）の中継。
type roomBridge struct {
	api         *backendAPI
	bridgeEpoch int64
	channelID   string
	ctx         context.Context
	cancel      context.CancelFunc
	broken      atomic.Bool

	// negotiate は Cloudflare のセッションの SDP のやり取りを 1 つずつにする。
	negotiate sync.Mutex
	mu        sync.Mutex
	lk        *lksdk.Room
	pc        *webrtc.PeerConnection
	cfSession string
	ready     chan struct{}
	// wanted は Cloudflare の人の声（track id → 話者）。
	wanted     map[string]string
	pulled     map[string]*pulledTrack
	pushed     map[string]*pushedTrack
	midSpeaker map[string]string
	lkSpeakers map[string]bool
}

func newRoomBridge(parent context.Context, api *backendAPI, bridgeEpoch int64, r roomState) *roomBridge {
	ctx, cancel := context.WithCancel(parent)
	return &roomBridge{
		api: api, bridgeEpoch: bridgeEpoch, channelID: r.ChannelID, ctx: ctx, cancel: cancel,
		ready:      make(chan struct{}),
		wanted:     map[string]string{},
		pulled:     map[string]*pulledTrack{},
		pushed:     map[string]*pushedTrack{},
		midSpeaker: map[string]string{},
		lkSpeakers: map[string]bool{},
	}
}

func (b *roomBridge) failed() bool { return b.broken.Load() }

func (b *roomBridge) log() *slog.Logger {
	return slog.With("channel_id", b.channelID)
}

func (b *roomBridge) fail(err error) {
	if b.ctx.Err() != nil {
		return
	}
	b.log().Warn("room bridge failed", "error", err)
	b.broken.Store(true)
}

func (b *roomBridge) start(r roomState) {
	if err := b.connect(r); err != nil {
		b.fail(err)
		return
	}
	close(b.ready)
	b.update(r)
}

func (b *roomBridge) connect(r roomState) error {
	pc, err := webrtc.NewPeerConnection(webrtc.Configuration{
		ICEServers:   []webrtc.ICEServer{{URLs: []string{"stun:stun.cloudflare.com:3478"}}},
		BundlePolicy: webrtc.BundlePolicyMaxBundle,
	})
	if err != nil {
		return err
	}
	pc.OnTrack(func(remote *webrtc.TrackRemote, receiver *webrtc.RTPReceiver) {
		mid := ""
		for _, t := range pc.GetTransceivers() {
			if t.Receiver() == receiver {
				mid = t.Mid()
			}
		}
		b.mu.Lock()
		speaker := b.midSpeaker[mid]
		b.mu.Unlock()
		if speaker == "" {
			return
		}
		go b.forwardToLiveKit(speaker, mid, remote)
	})
	pc.OnConnectionStateChange(func(s webrtc.PeerConnectionState) {
		if s == webrtc.PeerConnectionStateFailed {
			b.fail(errors.New("cloudflare peer connection failed"))
		}
	})
	b.pc = pc

	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	session, err := b.api.cfSession(ctx, b.channelID)
	if err != nil {
		return err
	}
	b.cfSession = session

	callback := &lksdk.RoomCallback{
		ParticipantCallback: lksdk.ParticipantCallback{
			OnTrackSubscribed: func(track *webrtc.TrackRemote, _ *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
				if track.Kind() != webrtc.RTPCodecTypeAudio || strings.HasPrefix(rp.Identity(), "bridge:") {
					return
				}
				go b.pushToCloudflare(rp.Identity(), track)
			},
		},
		OnDisconnected: func() { b.fail(errors.New("livekit disconnected")) },
	}
	room, err := lksdk.ConnectToRoomWithToken(r.LivekitURL, r.LivekitToken, callback, lksdk.WithAutoSubscribe(true))
	if err != nil {
		return err
	}
	b.lk = room
	return nil
}

// pushToCloudflare は LiveKit の話者の声を Cloudflare に push する。
func (b *roomBridge) pushToCloudflare(speaker string, track *webrtc.TrackRemote) {
	b.mu.Lock()
	allowed := b.lkSpeakers[speaker]
	b.mu.Unlock()
	if !allowed {
		// backend の一覧にまだいない（または LiveKit にいない）人は中継しない。
		b.log().Info("skipping unknown LiveKit speaker", "speaker", speaker)
		return
	}
	local, err := webrtc.NewTrackLocalStaticRTP(opusCapability, "audio", speaker)
	if err != nil {
		b.fail(err)
		return
	}
	mid, transceiver, err := b.publishLocal(speaker, local)
	if err != nil {
		b.log().Warn("push to Cloudflare failed", "speaker", speaker, "error", err)
		return
	}
	defer b.unpush(speaker, mid, transceiver)
	go drainRTCP(transceiver.Sender())
	for {
		pkt, _, err := track.ReadRTP()
		if err != nil || b.ctx.Err() != nil {
			return
		}
		stripExtensions(pkt)
		if err := local.WriteRTP(pkt); err != nil {
			return
		}
	}
}

func (b *roomBridge) publishLocal(speaker string, local *webrtc.TrackLocalStaticRTP) (string, *webrtc.RTPTransceiver, error) {
	b.negotiate.Lock()
	defer b.negotiate.Unlock()
	transceiver, err := b.pc.AddTransceiverFromTrack(local, webrtc.RTPTransceiverInit{Direction: webrtc.RTPTransceiverDirectionSendonly})
	if err != nil {
		return "", nil, err
	}
	offer, err := b.pc.CreateOffer(nil)
	if err != nil {
		return "", nil, err
	}
	gathered := webrtc.GatheringCompletePromise(b.pc)
	if err := b.pc.SetLocalDescription(offer); err != nil {
		return "", nil, err
	}
	<-gathered
	mid := transceiver.Mid()
	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	desc := b.pc.LocalDescription()
	res, err := b.api.cfPublish(ctx, b.channelID, b.bridgeEpoch, b.cfSession, speaker, sessionDescription{Type: "offer", SDP: desc.SDP}, mid)
	if err != nil {
		return "", nil, err
	}
	if res.SessionDescription == nil {
		return "", nil, errors.New("cloudflare returned no answer")
	}
	if err := b.pc.SetRemoteDescription(webrtc.SessionDescription{Type: webrtc.SDPTypeAnswer, SDP: res.SessionDescription.SDP}); err != nil {
		return "", nil, err
	}
	b.mu.Lock()
	b.pushed[speaker] = &pushedTrack{mid: mid, transceiver: transceiver}
	b.mu.Unlock()
	return mid, transceiver, nil
}

func (b *roomBridge) unpush(speaker, mid string, transceiver *webrtc.RTPTransceiver) {
	b.mu.Lock()
	if p, ok := b.pushed[speaker]; ok && p.mid == mid {
		delete(b.pushed, speaker)
	}
	b.mu.Unlock()
	if b.ctx.Err() != nil {
		return
	}
	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	_ = b.api.cfClose(ctx, b.cfSession, []string{mid})
	_ = transceiver.Stop()
}

func drainRTCP(sender *webrtc.RTPSender) {
	buf := make([]byte, 1500)
	for {
		if _, _, err := sender.Read(buf); err != nil {
			return
		}
	}
}

// forwardToLiveKit は Cloudflare の話者の声を LiveKit に publish する。トラック名は話者の session_id。
func (b *roomBridge) forwardToLiveKit(speaker, mid string, remote *webrtc.TrackRemote) {
	local, err := lksdk.NewLocalTrack(remote.Codec().RTPCodecCapability)
	if err != nil {
		b.fail(err)
		return
	}
	pub, err := b.lk.LocalParticipant.PublishTrack(local, &lksdk.TrackPublicationOptions{
		Name:   speaker,
		Source: livekit.TrackSource_MICROPHONE,
	})
	if err != nil {
		b.log().Warn("publish to LiveKit failed", "speaker", speaker, "error", err)
		return
	}
	defer func() { _ = b.lk.LocalParticipant.UnpublishTrack(pub.SID()) }()
	for {
		pkt, _, err := remote.ReadRTP()
		if err != nil || b.ctx.Err() != nil {
			return
		}
		stripExtensions(pkt)
		if err := local.WriteRTP(pkt, nil); err != nil {
			return
		}
	}
}

// update は backend の一覧に合わせる。Cloudflare の人の声を pull し、いなくなった人の声を止める。
func (b *roomBridge) update(r roomState) {
	select {
	case <-b.ready:
	default:
		// 接続中。start が終わったら最新の一覧で呼ばれる。
		return
	}
	if b.ctx.Err() != nil {
		return
	}
	b.mu.Lock()
	b.lkSpeakers = map[string]bool{}
	for _, s := range r.LivekitSessions {
		b.lkSpeakers[s] = true
	}
	b.wanted = map[string]string{}
	for _, t := range r.CloudflareTracks {
		if t.Source == "direct" {
			b.wanted[t.ID] = t.SessionID
		}
	}
	var toPull []string
	var toStop []*pulledTrack
	for id := range b.wanted {
		if _, ok := b.pulled[id]; !ok {
			toPull = append(toPull, id)
		}
	}
	for id, p := range b.pulled {
		if _, ok := b.wanted[id]; !ok {
			toStop = append(toStop, p)
			delete(b.pulled, id)
		}
	}
	for _, id := range toPull {
		b.pulled[id] = &pulledTrack{speaker: b.wanted[id]}
	}
	b.mu.Unlock()

	for _, p := range toStop {
		b.stopPulled(p)
	}
	if len(toPull) > 0 {
		go b.pull(toPull)
	}
}

func (b *roomBridge) pull(ids []string) {
	b.negotiate.Lock()
	defer b.negotiate.Unlock()
	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	res, err := b.api.cfPull(ctx, b.channelID, b.cfSession, ids)
	if err != nil {
		// 次の一覧の更新（最長 25 秒）で、もう一度 pull する。
		b.mu.Lock()
		for _, id := range ids {
			delete(b.pulled, id)
		}
		b.mu.Unlock()
		b.log().Warn("pull from Cloudflare failed", "error", err)
		return
	}
	b.mu.Lock()
	for _, t := range res.Tracks {
		if t.Mid == "" || t.ErrorCode != "" {
			continue
		}
		speaker := speakerOfTrackName(t.TrackName)
		b.midSpeaker[t.Mid] = speaker
		for _, p := range b.pulled {
			if p.speaker == speaker && p.mid == "" {
				p.mid = t.Mid
			}
		}
	}
	b.mu.Unlock()
	if !res.RequiresImmediateRenegotiation || res.SessionDescription == nil {
		return
	}
	if err := b.pc.SetRemoteDescription(webrtc.SessionDescription{Type: webrtc.SDPTypeOffer, SDP: res.SessionDescription.SDP}); err != nil {
		b.fail(err)
		return
	}
	answer, err := b.pc.CreateAnswer(nil)
	if err != nil {
		b.fail(err)
		return
	}
	gathered := webrtc.GatheringCompletePromise(b.pc)
	if err := b.pc.SetLocalDescription(answer); err != nil {
		b.fail(err)
		return
	}
	<-gathered
	if err := b.api.cfAnswer(ctx, b.cfSession, sessionDescription{Type: "answer", SDP: b.pc.LocalDescription().SDP}); err != nil {
		b.fail(err)
	}
}

func (b *roomBridge) stopPulled(p *pulledTrack) {
	if p.mid == "" {
		return
	}
	b.negotiate.Lock()
	defer b.negotiate.Unlock()
	for _, t := range b.pc.GetTransceivers() {
		if t.Mid() == p.mid {
			// 受信を止めると forwardToLiveKit の ReadRTP が終わり、LiveKit からも外れる。
			_ = t.Stop()
		}
	}
	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	_ = b.api.cfClose(ctx, b.cfSession, []string{p.mid})
}

func (b *roomBridge) close() {
	b.cancel()
	if b.lk != nil {
		b.lk.Disconnect()
	}
	if b.pc != nil {
		_ = b.pc.Close()
	}
}
