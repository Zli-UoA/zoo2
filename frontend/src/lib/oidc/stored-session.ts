import type { OidcTokens } from "./client";
import { isLoggedOut, type AuthStorage } from "./pending";

const SESSION_KEY = "zoo.oidc.session";

/** タブ内の再読み込みだけをまたぎ、別の認証設定のセッションは復元しない。 */
export function readStoredSession(
  storage: AuthStorage,
  binding: string,
  now: number,
): OidcTokens | null {
  try {
    if (isLoggedOut(storage)) {
      clearStoredSession(storage);
      return null;
    }
    const raw = storage.getItem(SESSION_KEY);
    if (!raw) return null;
    const record = JSON.parse(raw) as { binding?: unknown; tokens?: Partial<OidcTokens> };
    const tokens = record?.tokens;
    if (
      record?.binding === binding &&
      tokens &&
      typeof tokens.accessToken === "string" &&
      tokens.accessToken.length > 0 &&
      (tokens.refreshToken === null ||
        (typeof tokens.refreshToken === "string" && tokens.refreshToken.length > 0)) &&
      (tokens.idToken === null || typeof tokens.idToken === "string") &&
      typeof tokens.expiresAt === "number" &&
      Number.isFinite(tokens.expiresAt) &&
      (tokens.expiresAt > now || tokens.refreshToken)
    ) {
      return tokens as OidcTokens;
    }
  } catch {
    // 保存を使えないブラウザーでも、メモリ内の認証は続ける。
  }
  clearStoredSession(storage);
  return null;
}

export function writeStoredSession(
  storage: AuthStorage,
  binding: string,
  tokens: OidcTokens,
): void {
  try {
    storage.setItem(SESSION_KEY, JSON.stringify({ binding, tokens }));
  } catch {
    clearStoredSession(storage);
  }
}

export function clearStoredSession(storage: AuthStorage): void {
  try {
    storage.removeItem(SESSION_KEY);
  } catch {
    // sessionStorage が無効でもログアウト処理を妨げない。
  }
}
