/** sessionStorage 互換の最小インターフェース（テストで差し替え可能にする）。 */
export interface AuthStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/**
 * リダイレクトをまたいで保持する一時データ。トークンは含めない。
 *
 * トークン本体はメモリ保持が基本方針。ここに置くのは authorize リクエストの
 * 一時的な検証材料（state / nonce / code_verifier）と復帰先だけで、コールバック
 * 処理時に使い切りで削除する。タブを閉じれば消え、タブ間でも共有されない。
 */
export interface PendingAuthorization {
  state: string;
  nonce: string;
  verifier: string;
  returnTo: string;
  createdAt: number;
}

const PENDING_KEY = "zoo.oidc.pending";
const LOGGED_OUT_KEY = "zoo.oidc.loggedOut";
const REDIRECT_ATTEMPTS_KEY = "zoo.oidc.redirectAttempts";
const LAST_REDIRECT_KEY = "zoo.oidc.lastRedirectAt";
const AUTHORIZE_REDIRECT_KEY = "zoo.oidc.authorizeRedirectAt";

/**
 * 401 を契機にした再ログインの試行制限。
 *
 * リダイレクトをまたぐとモジュール変数のクールダウンはリセットされるため、
 * 「IdP ではログインできるがバックエンドが 401 を返す」設定不一致で
 * authorize と 401 を無限に往復し得る。試行を sessionStorage に残して
 * 上限を超えたらエラー画面に切り替える。
 */
export const REDIRECT_ATTEMPT_WINDOW_MS = 60_000;
export const MAX_REDIRECT_ATTEMPTS = 3;

/** 認可リクエストの許容滞留時間。これを過ぎたコールバックは無効として破棄する。 */
export const PENDING_TTL_MS = 10 * 60 * 1000;

export function savePending(storage: AuthStorage, pending: PendingAuthorization): void {
  storage.setItem(PENDING_KEY, JSON.stringify(pending));
}

/** 保存済みの認可リクエストを取り出す。使い切り（読み出しと同時に削除）。 */
export function takePending(
  storage: AuthStorage,
  now: number = Date.now(),
): PendingAuthorization | null {
  const raw = storage.getItem(PENDING_KEY);
  storage.removeItem(PENDING_KEY);
  if (!raw) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!isPendingAuthorization(parsed)) {
      return null;
    }
    if (now - parsed.createdAt > PENDING_TTL_MS) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

export function clearPending(storage: AuthStorage): void {
  storage.removeItem(PENDING_KEY);
}

function isPendingAuthorization(value: unknown): value is PendingAuthorization {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.state === "string" &&
    typeof record.nonce === "string" &&
    typeof record.verifier === "string" &&
    typeof record.returnTo === "string" &&
    typeof record.createdAt === "number"
  );
}

/**
 * ログアウト直後は IdP セッションが残っていても自動再ログインしない。
 * Kanidm は RP-initiated logout を提供しないため、これを立てないと
 * 「ログアウト → 即座にサイレント再ログイン」になり操作が無意味になる。
 */
export function markLoggedOut(storage: AuthStorage): void {
  storage.setItem(LOGGED_OUT_KEY, "1");
}

export function clearLoggedOut(storage: AuthStorage): void {
  storage.removeItem(LOGGED_OUT_KEY);
}

export function isLoggedOut(storage: AuthStorage): boolean {
  return storage.getItem(LOGGED_OUT_KEY) === "1";
}

/**
 * 401 起点の再ログイン試行を記録し、直近ウィンドウ内の回数を返す。
 * バックエンドで認証が通って 200 が返れば `clearRedirectAttempts` で消す。
 */
export function recordRedirectAttempt(storage: AuthStorage, now: number = Date.now()): number {
  const attempts = readRedirectAttempts(storage, now);
  attempts.push(now);
  storage.setItem(REDIRECT_ATTEMPTS_KEY, JSON.stringify(attempts));
  return attempts.length;
}

export function clearRedirectAttempts(storage: AuthStorage): void {
  storage.removeItem(REDIRECT_ATTEMPTS_KEY);
}

/**
 * 直近のリダイレクト開始時刻。リダイレクト直後の 401 は設定不一致による
 * ループの兆候なので、ページをまたいで検知できるよう sessionStorage に残す。
 */
export function markRedirectStarted(storage: AuthStorage, now: number = Date.now()): void {
  storage.setItem(LAST_REDIRECT_KEY, String(now));
}

export function readLastRedirectAt(storage: AuthStorage): number | null {
  const raw = storage.getItem(LAST_REDIRECT_KEY);
  if (!raw) {
    return null;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * authorize リダイレクトが進行中かどうか。
 *
 * 失効前の事前再認可では、遷移が始まる直前に発行済みのリクエストが 401 で
 * 返ることがある。これを設定不一致によるループと誤判定しないよう、
 * リダイレクト開始からコールバック処理までを記録する。
 */
export function markAuthorizeRedirectPending(storage: AuthStorage, now: number = Date.now()): void {
  storage.setItem(AUTHORIZE_REDIRECT_KEY, String(now));
}

export function clearAuthorizeRedirectPending(storage: AuthStorage): void {
  storage.removeItem(AUTHORIZE_REDIRECT_KEY);
}

export function isAuthorizeRedirectPending(
  storage: AuthStorage,
  now: number = Date.now(),
  maxAgeMs: number = PENDING_TTL_MS,
): boolean {
  const raw = storage.getItem(AUTHORIZE_REDIRECT_KEY);
  if (!raw) {
    return false;
  }
  const startedAt = Number(raw);
  if (!Number.isFinite(startedAt) || now - startedAt > maxAgeMs) {
    // リダイレクトが中断されたまま残った値は無効として捨てる。
    clearAuthorizeRedirectPending(storage);
    return false;
  }
  return true;
}

export type UnauthorizedRedirectState = "redirecting" | "recent" | "none";

/**
 * 401 を受けたときのリダイレクト状況を返す。
 *
 * `redirecting` は遷移前のリクエストが返っただけなので無視してよい。
 * `recent` はコールバック直後の 401 で、設定不一致によるループの疑いがある。
 */
export function readUnauthorizedRedirectState(
  storage: AuthStorage,
  now: number,
  recentMs: number,
): UnauthorizedRedirectState {
  // IdP から戻ったまま mount されない（bfcache）場合などに備え、古い記録は
  // リダイレクト中とみなさない。無視し続けると 401 のまま再ログインできない。
  if (isAuthorizeRedirectPending(storage, now, recentMs)) {
    return "redirecting";
  }
  const lastRedirectAt = readLastRedirectAt(storage);
  if (lastRedirectAt !== null && now - lastRedirectAt < recentMs) {
    return "recent";
  }
  return "none";
}

function readRedirectAttempts(storage: AuthStorage, now: number): number[] {
  const raw = storage.getItem(REDIRECT_ATTEMPTS_KEY);
  if (!raw) {
    return [];
  }
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(
      (value): value is number =>
        typeof value === "number" && now - value <= REDIRECT_ATTEMPT_WINDOW_MS,
    );
  } catch {
    return [];
  }
}
