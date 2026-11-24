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
/** 経路の候補を集め終わるまでの上限。Cloudflare は trickle ICE を受けないので、集め終わった SDP を送る。 */
export const CF_ICE_GATHERING_TIMEOUT_MS = 10_000;
/** answer を受けてから ICE と DTLS がつながるまでの上限。 */
export const CF_CONNECT_TIMEOUT_MS = 15_000;
/** pull に失敗したトラックを、一覧の更新を待たずに pull し直すまでの間隔（回ごとに倍）と回数の上限。 */
export const CF_PULL_RETRY_BASE_MS = 1000;
export const CF_PULL_RETRY_MAX_ATTEMPTS = 4;

/** Opus の DTX（無音のあいだ送らない）を有効にする。 */
export function enableOpusDtx(sdp: string): string {
  const opus = /a=rtpmap:(\d+) opus\/48000/i.exec(sdp);
  if (!opus) return sdp;
  const pt = opus[1];
  const fmtp = new RegExp(`(a=fmtp:${pt} [^\\r\\n]*)`);
  if (!fmtp.test(sdp)) return sdp;
  return sdp.replace(fmtp, (line) => (/usedtx=/.test(line) ? line : `${line};usedtx=1`));
}

/** トラックの Cloudflare での track 名（backend が publish で付ける名前）。 */
export function trackNameOf(track: SfuTrackInfo): string {
  return track.source === "bridge" ? `${track.session_id}-bridge` : track.session_id;
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
  /** pull 済みのトラック id → 話者。`rev` は公開の世代（同じ id で公開し直すと変わる）。 */
  private pulled = new Map<
    string,
    { speaker: string; trackName: string; rev: string; generation: number }
  >();
  /** 最新の一覧。pull の失敗後のやり直しで使う。 */
  private latestTracks: SfuTrackInfo[] = [];
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private retryAttempt = 0;
  private generation = 0;
  /** m-line（mid）→ 話者。 */
  private midSpeaker = new Map<string, string>();
  private closed = false;
  /** start() が終わって、つながったあと。これより前の失敗は start() の失敗として返す。 */
  private connected = false;
  /** close() で止める待機。 */
  private waiters = new Set<(err: Error) => void>();
  /** start() の途中で、交渉が続けられなくなった。 */
  private startFailure: Error | null = null;
  private pendingInitial: { tracks: SfuTrackInfo[]; version: number } | null;

  constructor(
    private readonly selfSessionId: string,
    private mic: MediaStreamTrack,
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
      if (this.closed || !this.connected) return;
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

  /** close() でも終わる待機。時間切れは失敗にする。 */
  private wait(
    subscribe: (resolve: () => void, reject: (err: Error) => void) => () => void,
    timeoutMs: number,
    timeoutMessage: string,
  ): Promise<void> {
    if (this.closed) return Promise.reject(new Error("Closed"));
    return new Promise<void>((resolve, reject) => {
      let unsubscribe = () => {};
      const finish = (err?: Error) => {
        clearTimeout(timer);
        this.waiters.delete(finish);
        unsubscribe();
        if (err) reject(err);
        else resolve();
      };
      const timer = setTimeout(() => finish(new Error(timeoutMessage)), timeoutMs);
      this.waiters.add(finish);
      unsubscribe = subscribe(
        () => finish(),
        (err) => finish(err),
      );
    });
  }

  /** 経路の候補を集め終わるのを待つ（Cloudflare には候補をあとから送れない）。 */
  private waitForIceGathering(): Promise<void> {
    return this.wait(
      (resolve) => {
        const check = () => {
          if (this.pc.iceGatheringState === "complete") resolve();
        };
        this.pc.addEventListener("icegatheringstatechange", check);
        check();
        return () => this.pc.removeEventListener("icegatheringstatechange", check);
      },
      CF_ICE_GATHERING_TIMEOUT_MS,
      "Cloudflare ICE gathering timed out",
    );
  }

  /** ICE と DTLS がつながるのを待つ。 */
  private waitForConnected(): Promise<void> {
    return this.wait(
      (resolve, reject) => {
        const check = () => {
          const state = this.pc.connectionState;
          if (state === "connected") resolve();
          else if (state === "failed" || state === "closed")
            reject(new Error(`Cloudflare connection ${state}`));
        };
        this.pc.addEventListener("connectionstatechange", check);
        check();
        return () => this.pc.removeEventListener("connectionstatechange", check);
      },
      CF_CONNECT_TIMEOUT_MS,
      "Cloudflare connection timed out",
    );
  }

  /** 候補を集め終えた local description の SDP。 */
  private async gatheredLocalSdp(): Promise<string> {
    await this.waitForIceGathering();
    const sdp = this.pc.localDescription?.sdp;
    if (!sdp) throw new Error("No local description");
    return sdp;
  }

  /** マイクを publish し、Cloudflare とつながるまで待つ。つながらなければ reject する。 */
  async start(): Promise<void> {
    const run = this.enqueue(async () => {
      const transceiver = this.pc.addTransceiver(this.mic, { direction: "sendonly" });
      this.sender = transceiver.sender;
      const offer = await this.pc.createOffer();
      await this.pc.setLocalDescription({ type: "offer", sdp: enableOpusDtx(offer.sdp ?? "") });
      const sdp = await this.gatheredLocalSdp();
      if (this.closed) return;
      const res = await this.api.publish({ type: "offer", sdp });
      if (this.closed) return;
      if (!res.sessionDescription) throw new Error("Cloudflare returned no answer");
      await this.pc.setRemoteDescription(res.sessionDescription as RTCSessionDescriptionInit);
    });
    const initial = this.pendingInitial;
    this.pendingInitial = null;
    if (initial) this.updateTracks(initial.tracks, initial.version);
    await run;
    if (this.closed) throw new Error("Closed");
    await this.waitForConnected();
    if (this.startFailure) throw this.startFailure;
    this.connected = true;
  }

  updateTracks(tracks: SfuTrackInfo[], version: number): void {
    if (version < this.version) return;
    this.version = version;
    this.latestTracks = tracks;
    // 新しい一覧で pull し直すので、やり直しの回数は数え直す。
    this.retryAttempt = 0;
    this.reconcile();
  }

  /** pull に失敗したトラックを、上限の回数まで間隔を空けて pull し直す（一覧の更新を待たない）。 */
  private scheduleRetry(): void {
    if (this.closed || this.retryTimer || this.retryAttempt >= CF_PULL_RETRY_MAX_ATTEMPTS) return;
    const delay = CF_PULL_RETRY_BASE_MS * 2 ** this.retryAttempt;
    this.retryAttempt++;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (!this.closed) this.reconcile();
    }, delay);
  }

  /** 最新の一覧（latestTracks）に合わせて、pull と停止をする。 */
  private reconcile(): void {
    const wanted = new Map<string, SfuTrackInfo>();
    for (const t of this.latestTracks) {
      if (t.session_id === this.selfSessionId) continue;
      wanted.set(t.id, t);
    }
    // Map は走査中に今の要素を消してもよい。
    for (const [id, entry] of this.pulled) {
      const want = wanted.get(id);
      // 一覧から消えたもの、別のセッションで公開し直された（rev が変わった）ものは、古い購読をやめる。
      // rev が変わったものは、下で新しい世代を pull する。
      if (!want || (want.rev ?? "") !== entry.rev) {
        this.pulled.delete(id);
        this.callbacks.onRemoteGone(entry.speaker);
      }
    }
    const toPull = [...wanted.keys()].filter((id) => !this.pulled.has(id));
    if (toPull.length === 0) return;
    // 登録ごとに世代を付ける。古い pull の失敗が、あとで同じ id に付けた登録を消さないようにする。
    const generation = ++this.generation;
    for (const id of toPull) {
      const t = wanted.get(id)!;
      this.pulled.set(id, {
        speaker: t.session_id,
        trackName: trackNameOf(t),
        rev: t.rev ?? "",
        generation,
      });
    }
    const isMine = (id: string) => this.pulled.get(id)?.generation === generation;
    // 失敗した id を、やり直し（scheduleRetry）か次の一覧の更新で pull し直せるようにする（自分の世代の登録だけ）。
    const release = (ids: string[]) => {
      let released = false;
      for (const id of ids) {
        if (isMine(id)) {
          this.pulled.delete(id);
          released = true;
        }
      }
      if (released) this.scheduleRetry();
    };
    void this.enqueue(async () => {
      // キューで待っているあいだに一覧から消えたもの・登録し直されたものは pull しない。
      const ids = toPull.filter(isMine);
      if (ids.length === 0) return;
      let res: CfTracksResponse;
      try {
        res = await this.api.pull(ids);
      } catch (err) {
        release(ids);
        throw err;
      }
      if (this.closed) return;
      // Cloudflare はトラックごとに失敗を返せる。成功したものは残し、失敗したもの・結果のないものだけ戻す。
      const succeeded = new Set<string>();
      for (const t of res.tracks ?? []) {
        if (t.mid && t.trackName && !t.errorCode) {
          this.midSpeaker.set(t.mid, speakerOfTrackName(t.trackName));
          succeeded.add(t.trackName);
        }
      }
      const failed = ids.filter((id) => !succeeded.has(this.pulled.get(id)?.trackName ?? ""));
      if (failed.length === 0) this.retryAttempt = 0;
      release(failed);
      if (res.requiresImmediateRenegotiation && res.sessionDescription) {
        try {
          await this.pc.setRemoteDescription(res.sessionDescription as RTCSessionDescriptionInit);
          const answer = await this.pc.createAnswer();
          await this.pc.setLocalDescription(answer);
          // answer も、候補を集め終えてから送る。
          const sdp = await this.gatheredLocalSdp();
          if (this.closed) return;
          await this.api.answer({ type: "answer", sdp });
        } catch (err) {
          // answer を返せないと Cloudflare のセッションが先へ進めないので、接続の失敗として扱う。
          if (!this.closed) {
            if (this.connected) this.callbacks.onFailed("cloudflare_renegotiation_failed");
            else this.startFailure ??= err instanceof Error ? err : new Error(String(err));
          }
          throw err;
        }
      }
    });
  }

  async replaceMicTrack(track: MediaStreamTrack): Promise<void> {
    if (!this.sender) {
      // まだ publish していなければ、publish するトラックを差し替える。
      this.mic = track;
      return;
    }
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
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    // Set は走査中に今の要素を消してもよい。
    for (const finish of this.waiters) finish(new Error("Closed"));
    const speakers = new Set<string>();
    for (const entry of this.pulled.values()) speakers.add(entry.speaker);
    for (const speaker of speakers) this.callbacks.onRemoteGone(speaker);
    this.pulled.clear();
    this.pc.close();
  }
}
