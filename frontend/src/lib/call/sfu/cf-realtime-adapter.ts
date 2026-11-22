/**
 * Cloudflare Realtime の SFU につなぐアダプタ（docs/calls-sfu.md）。
 *
 * PeerConnection は 1 本。マイクを push し、ほかの人の声を pull する。Cloudflare の API は backend が代理するので、
 * ここでは app secret も相手のセッション ID も扱わない（pull は backend が配るトラックの id で指定する）。
 */
import type { SfuAdapter, SfuAdapterCallbacks, SfuTrackInfo } from "./types";

export interface CfSessionDescription {
  type: string;
  sdp: string;
}

export interface CfTracksResponse {
  sessionDescription?: CfSessionDescription | null;
  requiresImmediateRenegotiation?: boolean;
  tracks?: { mid?: string | null; trackName?: string | null; errorCode?: string | null }[];
}

/** backend の代理 API（`/call/sfu/cf/*`）。 */
export interface CfRealtimeApi {
  publish(offer: CfSessionDescription): Promise<CfTracksResponse>;
  pull(trackIds: string[]): Promise<CfTracksResponse>;
  answer(answer: CfSessionDescription): Promise<void>;
}

const CF_ICE_SERVERS: RTCIceServer[] = [{ urls: "stun:stun.cloudflare.com:3478" }];

/** Opus の DTX（無音のあいだ送らない）を有効にする。 */
export function enableOpusDtx(sdp: string): string {
  const opus = /a=rtpmap:(\d+) opus\/48000/i.exec(sdp);
  if (!opus) return sdp;
  const pt = opus[1];
  const fmtp = new RegExp(`(a=fmtp:${pt} [^\\r\\n]*)`);
  if (!fmtp.test(sdp)) return sdp;
  return sdp.replace(fmtp, (line) => (/usedtx=/.test(line) ? line : `${line};usedtx=1`));
}

/** Cloudflare の track 名から話者の session_id を得る（bridge が出したものは `-bridge` が付く）。 */
export function speakerOfTrackName(trackName: string): string {
  return trackName.endsWith("-bridge") ? trackName.slice(0, -"-bridge".length) : trackName;
}

export class CfRealtimeAdapter implements SfuAdapter {
  readonly kind = "cloudflare" as const;
  private pc: RTCPeerConnection;
  private sender: RTCRtpSender | null = null;
  private queue: Promise<void> = Promise.resolve();
  private version = -1;
  /** pull 済みのトラック id → 話者。 */
  private pulled = new Map<string, string>();
  /** m-line（mid）→ 話者。 */
  private midSpeaker = new Map<string, string>();
  private closed = false;
  private pendingInitial: { tracks: SfuTrackInfo[]; version: number } | null;

  constructor(
    private readonly selfSessionId: string,
    private readonly mic: MediaStreamTrack,
    private readonly api: CfRealtimeApi,
    private readonly callbacks: SfuAdapterCallbacks,
    initial: { tracks: SfuTrackInfo[]; version: number },
  ) {
    this.pc = new RTCPeerConnection({ iceServers: CF_ICE_SERVERS, bundlePolicy: "max-bundle" });
    this.pc.addEventListener("track", (event) => {
      const mid = event.transceiver.mid;
      const speaker = mid ? this.midSpeaker.get(mid) : undefined;
      if (!speaker) return;
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      this.callbacks.onRemoteAudio(speaker, stream);
    });
    this.pc.addEventListener("connectionstatechange", () => {
      if (this.closed) return;
      if (this.pc.connectionState === "failed") this.callbacks.onFailed("cloudflare_failed");
    });
    this.pendingInitial = initial;
  }

  private enqueue(task: () => Promise<void>): Promise<void> {
    const run = this.queue.then(async () => {
      if (this.closed) return;
      await task();
    });
    this.queue = run.catch((err) => {
      if (!this.closed) console.error("Cloudflare Realtime negotiation failed", err);
    });
    return run;
  }

  start(): Promise<void> {
    const run = this.enqueue(async () => {
      const transceiver = this.pc.addTransceiver(this.mic, { direction: "sendonly" });
      this.sender = transceiver.sender;
      const offer = await this.pc.createOffer();
      const sdp = enableOpusDtx(offer.sdp ?? "");
      await this.pc.setLocalDescription({ type: "offer", sdp });
      const res = await this.api.publish({ type: "offer", sdp });
      if (this.closed) return;
      if (!res.sessionDescription) throw new Error("Cloudflare returned no answer");
      await this.pc.setRemoteDescription(res.sessionDescription as RTCSessionDescriptionInit);
    });
    const initial = this.pendingInitial;
    this.pendingInitial = null;
    if (initial) this.updateTracks(initial.tracks, initial.version);
    return run;
  }

  updateTracks(tracks: SfuTrackInfo[], version: number): void {
    if (version < this.version) return;
    this.version = version;
    const wanted = new Map<string, string>();
    for (const t of tracks) {
      if (t.session_id === this.selfSessionId) continue;
      wanted.set(t.id, t.session_id);
    }
    // Map は走査中に今の要素を消してもよい。
    for (const [id, speaker] of this.pulled) {
      if (!wanted.has(id)) {
        this.pulled.delete(id);
        this.callbacks.onRemoteGone(speaker);
      }
    }
    const toPull = [...wanted.keys()].filter((id) => !this.pulled.has(id));
    if (toPull.length === 0) return;
    for (const id of toPull) this.pulled.set(id, wanted.get(id)!);
    void this.enqueue(async () => {
      // キューで待っているあいだに一覧から消えたものは pull しない。
      const ids = toPull.filter((id) => this.pulled.has(id));
      if (ids.length === 0) return;
      let res: CfTracksResponse;
      try {
        res = await this.api.pull(ids);
      } catch (err) {
        // 次の一覧の更新で、もう一度 pull する。
        for (const id of ids) this.pulled.delete(id);
        throw err;
      }
      if (this.closed) return;
      for (const t of res.tracks ?? []) {
        if (t.mid && t.trackName && !t.errorCode) {
          this.midSpeaker.set(t.mid, speakerOfTrackName(t.trackName));
        }
      }
      if (res.requiresImmediateRenegotiation && res.sessionDescription) {
        await this.pc.setRemoteDescription(res.sessionDescription as RTCSessionDescriptionInit);
        const answer = await this.pc.createAnswer();
        await this.pc.setLocalDescription(answer);
        await this.api.answer({ type: "answer", sdp: answer.sdp ?? "" });
      }
    });
  }

  async replaceMicTrack(track: MediaStreamTrack): Promise<void> {
    if (!this.sender) throw new Error("Microphone is not published yet");
    await this.sender.replaceTrack(track);
  }

  async setMuted(muted: boolean): Promise<void> {
    // DTX が有効なので、無効にしたトラック（無音）はほとんど送られない。
    const track = this.sender?.track;
    if (track) track.enabled = !muted;
  }

  peerConnections(): RTCPeerConnection[] {
    return [this.pc];
  }

  close(): void {
    this.closed = true;
    for (const speaker of new Set(this.pulled.values())) this.callbacks.onRemoteGone(speaker);
    this.pulled.clear();
    this.pc.close();
  }
}
