export type TokenProvider = () => Promise<string | null> | string | null;

/** 401 を受けたときの再ログイン導線。OIDC 有効時のみ AuthGate が登録する。 */
export type UnauthorizedHandler = () => void;

/** 認証付きリクエストが成功した通知。再ログイン試行のカウンタ消去に使う。 */
export type AuthorizedHandler = () => void;

let tokenProvider: TokenProvider = () => null;
let unauthorizedHandler: UnauthorizedHandler | null = null;
let authorizedHandler: AuthorizedHandler | null = null;

export function setTokenProvider(provider: TokenProvider): void {
  tokenProvider = provider;
}

export function resetTokenProvider(): void {
  tokenProvider = () => null;
}

export async function getToken(): Promise<string | null> {
  return await tokenProvider();
}

export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

/**
 * API / Shape が 401 を返したことを通知する。OIDC 未設定（dev）では何もしないので、
 * 既存の X-Dev-User 運用に影響しない。
 */
export function notifyUnauthorized(): void {
  unauthorizedHandler?.();
}

export function setAuthorizedHandler(handler: AuthorizedHandler | null): void {
  authorizedHandler = handler;
}

/** 認証付きリクエストが成功したことを通知する（dev では何もしない）。 */
export function notifyAuthorized(): void {
  authorizedHandler?.();
}
