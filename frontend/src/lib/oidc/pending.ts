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
