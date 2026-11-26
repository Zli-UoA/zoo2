/** 通話の SFU（docs/calls-sfu.md）のクライアント側の型。 */

export type RoomMode = "mesh" | "sfu";

/** Cloudflare Realtime にある音声トラック。`id` を pull に渡す。 */
export interface SfuTrackInfo {
  id: string;
  session_id: string;
  source: "direct" | "bridge";
  /**
   * 公開の世代（backend が Cloudflare のセッションと track 名から作る）。同じ `id` のまま別のセッションで
   * 公開し直すと変わるので、変わったら pull し直す。古い backend は送らないので省略できる。
   */
  rev?: string;
}

export type SfuJoinResponse =
  | {
      sfu: "livekit";
      url: string;
      token: string;
      epoch: number;
      version: number;
      tracks: SfuTrackInfo[];
    }
  | {
      sfu: "cloudflare";
      epoch: number;
      version: number;
      tracks: SfuTrackInfo[];
    };

export interface SfuAdapterCallbacks {
  /** 話者（call の session_id）の音声が届いた。 */
  onRemoteAudio: (sessionId: string, stream: MediaStream) => void;
  /** 話者の音声が止まった。 */
  onRemoteGone: (sessionId: string) => void;
  /** 接続できなかった・切れた。LiveKit なら Cloudflare でやり直す。 */
  onFailed: (reason: string) => void;
}

/** LiveKit と Cloudflare Realtime で同じ形にそろえたアダプタ。 */
export interface SfuAdapter {
  readonly kind: "livekit" | "cloudflare";
  /** 接続してマイクを publish する。 */
  start(): Promise<void>;
  /** Cloudflare のトラック一覧が変わった（LiveKit では何もしない）。 */
  updateTracks(tracks: SfuTrackInfo[], version: number): void;
  /** マイクのトラックを差し替える。 */
  replaceMicTrack(track: MediaStreamTrack): Promise<void>;
  setMuted(muted: boolean): Promise<void>;
  /** 診断用。見えるものだけ。 */
  peerConnections(): RTCPeerConnection[];
  close(): void;
}

export function isSfuTrackInfo(value: unknown): value is SfuTrackInfo {
  if (!value || typeof value !== "object") return false;
  const t = value as Record<string, unknown>;
  return (
    typeof t.id === "string" &&
    typeof t.session_id === "string" &&
    (t.source === "direct" || t.source === "bridge") &&
    (t.rev === undefined || typeof t.rev === "string")
  );
}

export function isSfuJoinResponse(value: unknown): value is SfuJoinResponse {
  if (!value || typeof value !== "object") return false;
  const r = value as Record<string, unknown>;
  if (typeof r.epoch !== "number" || typeof r.version !== "number") return false;
  if (!Array.isArray(r.tracks) || !r.tracks.every(isSfuTrackInfo)) return false;
  if (r.sfu === "cloudflare") return true;
  return r.sfu === "livekit" && typeof r.url === "string" && typeof r.token === "string";
}
