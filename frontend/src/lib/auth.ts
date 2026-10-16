export type TokenProvider = () => Promise<string | null> | string | null;

/** 401 を受けたときの再ログイン導線。OIDC 有効時のみ AuthGate が登録する。 */
export type UnauthorizedHandler = () => void;

let tokenProvider: TokenProvider = () => null;
let unauthorizedHandler: UnauthorizedHandler | null = null;

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
