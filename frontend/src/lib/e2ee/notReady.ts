import { ApiError } from "../api/client";
import type { E2eeDeviceStatus } from "./deviceManager";

/**
 * このタブで暗号ストアを開けていない理由。設定の「暗号化」タブの上に出す。
 *
 * - `unsupported`：暗号ストアを置けないブラウザー（OPFS、Worker、Web Locks がない）。
 * - `signedOut`：ログインしているユーザーがいない（ユーザーの取得が終わり、失敗もしていない）。
 * - `userUnavailable`：ログインしているユーザーの取得に失敗した（5xx、タイムアウトなど。401 ではない）。
 * - `offline`：オフライン。端末の登録と鍵の補充ができない。
 * - `otherTab`：ほかのタブが暗号ストアを開いている（Web Locks を持っている）。そのタブを閉じると引き継ぐ。
 * - `starting`：暗号ストアを開き、端末を登録しているところ。
 * - `deviceLimit`：端末が 20 台の上限で、登録を拒否された。同じタブの端末の一覧で削除してもらう。
 * - `failed`：開くか登録に失敗し、間隔をあけて試し直している。`source` で原因の場所を分ける。
 */
export type E2eeNotReadyReason =
  | { kind: "unsupported" }
  | { kind: "signedOut" }
  | { kind: "userUnavailable" }
  | { kind: "offline" }
  | { kind: "otherTab" }
  | { kind: "starting" }
  | { kind: "deviceLimit" }
  | { kind: "failed"; failure: E2eeStartFailure };

/** 理由がまだ分からないときの既定（開いているところ）。 */
export const NOT_READY_STARTING: E2eeNotReadyReason = { kind: "starting" };

/** 起動（暗号ストアを開く、端末の登録、鍵の補充）に失敗した理由。 */
export type E2eeStartFailure =
  /** サーバーがエラーを返した。`code` は応答の本文の `code`（あれば）。 */
  | { source: "server"; status: number; code: string | null }
  /** サーバーにつながらなかった（fetch の失敗）。 */
  | { source: "network" }
  /** この端末で暗号ストアを開けなかった（Worker、wasm、OPFS の失敗）。`name` はエラーの名前。 */
  | { source: "local"; name: string | null };

/** Web Locks の状態。`otherTab` は、ほかのタブがロックを持っていて待っている。 */
export type CryptoLockState = "idle" | "otherTab" | "held";

export function describeStartFailure(error: unknown): E2eeStartFailure {
  if (error instanceof ApiError) {
    const code = (error.body as { code?: unknown } | null | undefined)?.code;
    return { source: "server", status: error.status, code: typeof code === "string" ? code : null };
  }
  // fetch は、つながらないときに TypeError を投げる。
  if (error instanceof TypeError) return { source: "network" };
  return describeLocalFailure(error);
}

/**
 * この端末での失敗として扱う（エラーの名前だけを残す）。Worker を作るときの例外は、種類に
 * かかわらずこれにする。`Worker()` は、CSP の Trusted Types などで TypeError も投げるため。
 */
export function describeLocalFailure(error: unknown): E2eeStartFailure {
  const name = error instanceof Error && error.name && error.name !== "Error" ? error.name : null;
  return { source: "local", name };
}

export interface NotReadyInput {
  supported: boolean;
  userId: string | null;
  /** ログインしているユーザーを取得しているところ（`/api/users/me`）。 */
  userLoading: boolean;
  /** ユーザーの取得が 401 以外で失敗した。 */
  userError: boolean;
  isOnline: boolean;
  status: E2eeDeviceStatus | "unsupported" | "idle";
  ready: boolean;
  lockState: CryptoLockState;
  deviceLimit: boolean;
  startFailure: E2eeStartFailure | null;
}

/** 暗号ストアを開けていない理由。開けていれば `null`。 */
export function notReadyReason(input: NotReadyInput): E2eeNotReadyReason | null {
  if (!input.supported || input.status === "unsupported") return { kind: "unsupported" };
  if (input.userId && input.status === "ready" && input.ready) return null;
  // オフラインで起動すると、ログイン中のユーザーを確かめられず `userId` がない（OIDC）。
  // ログインし直しを案内しないよう、ユーザーより先にオフラインかを見る。
  if (!input.isOnline) return { kind: "offline" };
  if (!input.userId) {
    // 取得中や一時的な失敗のときに、ログインし直しを案内しない（401 なら AuthGate が案内する）。
    if (input.userLoading) return { kind: "starting" };
    if (input.userError) return { kind: "userUnavailable" };
    return { kind: "signedOut" };
  }
  if (input.lockState === "otherTab") return { kind: "otherTab" };
  if (input.deviceLimit) return { kind: "deviceLimit" };
  if (input.startFailure) return { kind: "failed", failure: input.startFailure };
  return { kind: "starting" };
}
