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
	// mu は下のフィールドを守る。lk と pc は、close との取り合いを避けるため mu の下で置く。
	mu        sync.Mutex
	lk        *lksdk.Room
	pc        *webrtc.PeerConnection
	cfSession string
	ready     chan struct{}
	// latest は最新の一覧。接続中に届いた分も、接続が終わったら sync で反映する。
	latest roomState
	// wanted は Cloudflare の人の声（track id → 話者）。
	wanted     map[string]string
	pulled     map[string]*pulledTrack
	pushed     map[string]*pushedTrack
	midSpeaker map[string]string
	lkSpeakers map[string]bool
	// parked は、一覧にまだいない LiveKit の話者の購読済みトラック。一覧に載ったら push する。
	parked map[string][]*webrtc.TrackRemote
}

func newRoomBridge(parent context.Context, api *backendAPI, bridgeEpoch int64, r roomState) *roomBridge {
	ctx, cancel := context.WithCancel(parent)
	b := &roomBridge{
		api: api, bridgeEpoch: bridgeEpoch, channelID: r.ChannelID, ctx: ctx, cancel: cancel,
		ready:      make(chan struct{}),
		latest:     r,
		wanted:     map[string]string{},
		pulled:     map[string]*pulledTrack{},
		pushed:     map[string]*pushedTrack{},
		midSpeaker: map[string]string{},
		lkSpeakers: map[string]bool{},
		parked:     map[string][]*webrtc.TrackRemote{},
	}
	// 購読より先に、最初の一覧の LiveKit の話者を入れておく。
	b.setLivekitSpeakersLocked(r)
	return b
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
	b.sync()
}

// own は、接続中に作ったものを close と取り合わずに置く。close のあとなら置かずに閉じ、false を返す。
func (b *roomBridge) own(set, closeNow func()) bool {
	b.mu.Lock()
	if b.ctx.Err() != nil {
		b.mu.Unlock()
		closeNow()
		return false
	}
	set()
	b.mu.Unlock()
	return true
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
	if !b.own(func() { b.pc = pc }, func() { _ = pc.Close() }) {
		return context.Canceled
	}

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
			OnTrackUnsubscribed: func(track *webrtc.TrackRemote, _ *lksdk.RemoteTrackPublication, rp *lksdk.RemoteParticipant) {
				b.unpark(rp.Identity(), track)
			},
		},
		OnDisconnected: func() { b.fail(errors.New("livekit disconnected")) },
	}
	// close で b.ctx が取り消されたら、接続の途中でも止める。
	room := lksdk.NewRoom(callback)
	if err := room.JoinWithContextAndToken(b.ctx, r.LivekitURL, r.LivekitToken, lksdk.WithAutoSubscribe(true)); err != nil {
		return err
	}
	// 接続し終える前に close されていたら、ここで切る。
	if !b.own(func() { b.lk = room }, room.Disconnect) {
		return context.Canceled
	}
	return nil
}

// pushToCloudflare は LiveKit の話者の声を Cloudflare に push する。
func (b *roomBridge) pushToCloudflare(speaker string, track *webrtc.TrackRemote) {
	b.mu.Lock()
	allowed := b.lkSpeakers[speaker]
	if !allowed {
		// backend の一覧にまだいない人の声は中継しない。トラックは取っておき、一覧に載ったら sync が push する。
		b.parked[speaker] = append(b.parked[speaker], track)
	}
	b.mu.Unlock()
	if !allowed {
		b.log().Info("holding LiveKit speaker until listed", "speaker", speaker)
		return
	}
	b.relayToCloudflare(speaker, track)
}

// unpark は、購読をやめたトラックを取っておいた分から外す。
func (b *roomBridge) unpark(speaker string, track *webrtc.TrackRemote) {
	b.mu.Lock()
	defer b.mu.Unlock()
	tracks := b.parked[speaker]
	for i, t := range tracks {
		if t == track {
			tracks = append(tracks[:i:i], tracks[i+1:]...)
			break
		}
	}
	if len(tracks) == 0 {
		delete(b.parked, speaker)
	} else {
		b.parked[speaker] = tracks
	}
}

func (b *roomBridge) relayToCloudflare(speaker string, track *webrtc.TrackRemote) {
	local, err := webrtc.NewTrackLocalStaticRTP(opusCapability, "audio", speaker)
	if err != nil {
		b.fail(err)
		return
	}
	mid, transceiver, err := b.publishLocal(speaker, local)
	if err != nil {
		// transceiver の追加や SetLocalDescription まで進んでいることがあり、やり直すと Cloudflare 側と食い違う。
		// 中継を作り直させる（supervisor が次の一覧で作り直す）。
		b.fail(err)
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
		// b.pulled は取得済みのまま残るので、中継を作り直させる（supervisor が次の一覧で作り直す）。
		b.fail(err)
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

// update は backend の一覧を受け取る。接続中でも LiveKit の話者の一覧は入れる。
func (b *roomBridge) update(r roomState) {
	b.mu.Lock()
	b.latest = r
	b.setLivekitSpeakersLocked(r)
	b.mu.Unlock()
	b.sync()
}

// sync は最新の一覧（b.latest）に合わせる。Cloudflare の人の声を pull し、いなくなった人の声を止め、
// 一覧に載った LiveKit の話者の取っておいたトラックを push する。
func (b *roomBridge) sync() {
	if b.ctx.Err() != nil {
		return
	}
	b.mu.Lock()
	select {
	case <-b.ready:
	default:
		// 接続中。start が終わったら呼ばれる。
		b.mu.Unlock()
		return
	}
	r := b.latest
	var toPush []parkedTrack
	for speaker, tracks := range b.parked {
		if !b.lkSpeakers[speaker] {
			continue
		}
		for _, t := range tracks {
			toPush = append(toPush, parkedTrack{speaker: speaker, track: t})
		}
		delete(b.parked, speaker)
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

	for _, p := range toPush {
		go b.relayToCloudflare(p.speaker, p.track)
	}
	for _, p := range toStop {
		b.stopPulled(p)
	}
	if len(toPull) > 0 {
		go b.pull(toPull)
	}
}

type parkedTrack struct {
	speaker string
	track   *webrtc.TrackRemote
}

func (b *roomBridge) setLivekitSpeakersLocked(r roomState) {
	b.lkSpeakers = map[string]bool{}
	for _, s := range r.LivekitSessions {
		b.lkSpeakers[s] = true
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
	orphans := b.assignPulled(ids, res.Tracks)
	// 応答を待つあいだに一覧から外れた分は、受信の準備ができてから止める。
	defer func() {
		if len(orphans) > 0 && b.ctx.Err() == nil {
			b.stopMidsLocked(orphans)
		}
	}()
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

// assignPulled は pull の結果を b.pulled に入れる。
//   - 成功した track の mid は、同じ話者の、まだ mid のない ids の予約に入れる。
//   - 予約がもう無い（応答を待つあいだに一覧から外れた）mid は返す。呼び出し側が止める。
//   - 失敗した（errorCode がある、または結果が無い）ids の予約は外す。次の一覧の更新で、もう一度 pull する。
func (b *roomBridge) assignPulled(ids []string, tracks []trackResult) (orphans []string) {
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, t := range tracks {
		if t.Mid == "" || t.ErrorCode != "" {
			if t.ErrorCode != "" {
				b.log().Warn("pull from Cloudflare failed for a track", "track_name", t.TrackName, "error_code", t.ErrorCode)
			}
			continue
		}
		speaker := speakerOfTrackName(t.TrackName)
		assigned := false
		for _, id := range ids {
			if p := b.pulled[id]; p != nil && p.speaker == speaker && p.mid == "" {
				p.mid = t.Mid
				assigned = true
				break
			}
		}
		if !assigned {
			orphans = append(orphans, t.Mid)
			continue
		}
		b.midSpeaker[t.Mid] = speaker
	}
	for _, id := range ids {
		if p := b.pulled[id]; p != nil && p.mid == "" {
			delete(b.pulled, id)
		}
	}
	return orphans
}

func (b *roomBridge) stopPulled(p *pulledTrack) {
	if p.mid == "" {
		// pull の応答を待っている。応答が来たら pull が止める（assignPulled）。
		return
	}
	b.negotiate.Lock()
	defer b.negotiate.Unlock()
	b.stopMidsLocked([]string{p.mid})
}

// stopMidsLocked は受信を止め、Cloudflare 側も閉じる。b.negotiate を持って呼ぶ。
func (b *roomBridge) stopMidsLocked(mids []string) {
	stop := map[string]bool{}
	for _, mid := range mids {
		stop[mid] = true
	}
	for _, t := range b.pc.GetTransceivers() {
		if stop[t.Mid()] {
			// 受信を止めると forwardToLiveKit の ReadRTP が終わり、LiveKit からも外れる。
			_ = t.Stop()
		}
	}
	b.mu.Lock()
	for _, mid := range mids {
		delete(b.midSpeaker, mid)
	}
	b.mu.Unlock()
	ctx, cancel := context.WithTimeout(b.ctx, negotiateTimeout)
	defer cancel()
	_ = b.api.cfClose(ctx, b.cfSession, mids)
}

func (b *roomBridge) close() {
	// 取り消しと lk・pc の読み出しを mu の下で行う。connect は mu の下で取り消しを確かめてから置く（own）ので、
	// close のあとに接続し終えたものは connect が閉じる。
	b.mu.Lock()
	b.cancel()
	lk, pc := b.lk, b.pc
	b.mu.Unlock()
	if lk != nil {
		lk.Disconnect()
	}
	if pc != nil {
		_ = pc.Close()
	}
}
