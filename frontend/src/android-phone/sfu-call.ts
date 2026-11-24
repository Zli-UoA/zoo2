/**
 * Android の DM 通話の SFU モード（docs/calls-sfu.md）。
 *
 * WebView は外へ接続できない（CSP の connect-src 'none'）ので、LiveKit は使わず Cloudflare Realtime だけを使う。
 * backend の API は Kotlin が代わりに呼ぶ（`sfu_request` を出し、`rtc.sfu.reply` で結果を受け取る）。
 */
import { CfRealtimeAdapter, type CfTracksResponse } from "../lib/call/sfu/cf-realtime-adapter";
import { isSfuJoinResponse, isSfuTrackInfo, type SfuTrackInfo } from "../lib/call/sfu/types";

type Emit = (event: string, payload?: unknown) => void;

/** Kotlin に許している SFU の API。 */
export type SfuRequestOp = "join" | "cf/publish" | "cf/pull" | "cf/answer";

export class AndroidSfuCall {
  private adapter: CfRealtimeAdapter | null = null;
  private replies = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  private serial = 0;
  private audios = new Map<string, HTMLAudioElement>();
  private others = new Set<string>();
  private hadRemote = false;
  private closed = false;
  private latest: { tracks: SfuTrackInfo[]; version: number } | null = null;

  constructor(
    private readonly session: string,
    private readonly mic: MediaStreamTrack,
    private readonly emit: Emit,
    participants: { session_id: string }[],
  ) {
    for (const p of participants) if (p.session_id !== session) this.others.add(p.session_id);
    this.hadRemote = this.others.size > 0;
  }

  private request(op: SfuRequestOp, body: Record<string, unknown>): Promise<unknown> {
    const id = ++this.serial;
    return new Promise((resolve, reject) => {
      this.replies.set(id, { resolve, reject });
      this.emit("sfu_request", { id, op, body });
    });
  }

  /** Kotlin からの API の結果。 */
  reply(value: Record<string, unknown>) {
    const id = typeof value.id === "number" ? value.id : -1;
    const pending = this.replies.get(id);
    if (!pending) return;
    this.replies.delete(id);
    if (value.ok === true) pending.resolve(value.data);
    else pending.reject(new Error("SFU request failed"));
  }

  async start() {
    const joined = await this.request("join", { cloudflare_only: true });
    if (this.closed) return;
    if (!isSfuJoinResponse(joined) || joined.sfu !== "cloudflare") throw new Error("Invalid join");
    const initial =
      this.latest && this.latest.version > joined.version
        ? this.latest
        : { tracks: joined.tracks, version: joined.version };
    this.adapter = new CfRealtimeAdapter(
      this.session,
      this.mic,
      {
        publish: (offer) => this.request("cf/publish", { offer }) as Promise<CfTracksResponse>,
        pull: (trackIds) =>
          this.request("cf/pull", { track_ids: trackIds }) as Promise<CfTracksResponse>,
        answer: async (answer) => {
          await this.request("cf/answer", { answer });
        },
      },
      {
        onRemoteAudio: (sid, stream) => this.attach(sid, stream),
        onRemoteGone: (sid) => this.detach(sid),
        onFailed: () => {
          if (!this.closed) this.emit("failed");
        },
      },
      initial,
      // join の応答待ちのあいだのミュートは mic.enabled に入っている。
      { muted: !this.mic.enabled },
    );
    // start() は Cloudflare と ICE・DTLS までつながってから終わる（つながらなければ reject）。
    await this.adapter.start();
    if (!this.closed) this.emit("connected");
  }

  private attach(sid: string, stream: MediaStream) {
    this.detach(sid);
    const audio = document.createElement("audio");
    audio.srcObject = stream;
    audio.autoplay = true;
    this.audios.set(sid, audio);
    void audio.play().catch(() => {});
  }

  private detach(sid: string) {
    const audio = this.audios.get(sid);
    if (!audio) return;
    this.audios.delete(sid);
    audio.pause();
    audio.srcObject = null;
  }

  /** SSE のイベント（room_state 以外）。 */
  event(event: string, data: Record<string, unknown>) {
    if (event === "sfu_tracks") {
      const version = typeof data.version === "number" ? data.version : -1;
      const tracks = Array.isArray(data.tracks) ? data.tracks.filter(isSfuTrackInfo) : [];
      if (!this.latest || version > this.latest.version) this.latest = { tracks, version };
      this.adapter?.updateTracks(tracks, version);
    } else if (event === "participant_joined") {
      const p = data.participant as { session_id?: string } | undefined;
      if (p?.session_id && p.session_id !== this.session) {
        this.others.add(p.session_id);
        this.hadRemote = true;
      }
    } else if (event === "participant_left") {
      const id = data.session_id as string;
      this.others.delete(id);
      this.detach(id);
      // DM で相手が全員抜けたら、通話を終える（mesh と同じ）。
      if (this.hadRemote && this.others.size === 0) this.emit("ended");
    }
  }

  mute(muted: boolean) {
    // join の応答待ちでアダプタがまだなくても、送るマイク（複製）に反映する。
    this.mic.enabled = !muted;
    void this.adapter?.setMuted(muted);
  }

  /** 相手の声の大きさ（0〜1）。Cloudflare の PeerConnection の受信の統計から読む。 */
  async remoteAudioLevel(): Promise<number> {
    const pc = this.adapter?.peerConnections()[0];
    if (!pc || pc.connectionState !== "connected") return 0;
    let level = 0;
    try {
      (await pc.getStats()).forEach((report) => {
        if (
          report.type === "inbound-rtp" &&
          report.kind === "audio" &&
          Number.isFinite(report.audioLevel)
        )
          level = Math.max(level, report.audioLevel as number);
      });
    } catch {
      return 0;
    }
    return Math.min(1, level);
  }

  close() {
    this.closed = true;
    this.adapter?.close();
    this.adapter = null;
    for (const sid of this.audios.keys()) this.detach(sid);
    for (const pending of this.replies.values()) pending.reject(new Error("Closed"));
    this.replies.clear();
    this.mic.stop();
  }
}
