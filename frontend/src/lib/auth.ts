export type TokenProvider = () => Promise<string | null> | string | null;

/** 401 を受けたときの再ログイン導線。OIDC 有効時のみ AuthGate が登録する。 */
export type UnauthorizedHandler = () => void;

/** 認証付きリクエストが成功した通知。再ログイン試行のカウンタ消去に使う。 */
export type AuthorizedHandler = () => void;

export type AuthSessionEndReason = "logout" | "reset";
export type AuthSessionLifecycleEvent = AuthSessionEndReason | "reauthorize" | "logoutBlocked";
export type AuthSessionEndListener = (reason: AuthSessionLifecycleEvent) => boolean | void;

let tokenProvider: TokenProvider = () => null;
let unauthorizedHandler: UnauthorizedHandler | null = null;
let authorizedHandler: AuthorizedHandler | null = null;
let authSessionGeneration = 0;
let authRequestGeneration = 0;
const authSessionEndListeners = new Set<AuthSessionEndListener>();

export function getAuthSessionGeneration(): number {
  return authSessionGeneration;
}

/** Requests started before a login/reauthorization operation must not affect it later. */
export function getAuthRequestGeneration(): number {
  return authRequestGeneration;
}

export function subscribeAuthSessionEnded(listener: AuthSessionEndListener): () => void {
  authSessionEndListeners.add(listener);
  return () => authSessionEndListeners.delete(listener);
}

/** 認証セッションが破棄された共通経路。遅延レスポンスの世代判定にも使う。 */
export function notifyAuthSessionEnded(reason: AuthSessionEndReason): boolean {
  authSessionGeneration += 1;
  authRequestGeneration += 1;
  return notifyAuthSessionListeners(reason);
}

/** Persistent cleanup was not confirmed, so callers must keep the user on this page. */
export function notifyAuthSessionLogoutBlocked(): void {
  authRequestGeneration += 1;
  notifyAuthSessionListeners("logoutBlocked");
}

/**
 * 期限前の再認可でauthorizeへ遷移する直前に通知する。
 * 認証世代は継続したまま、ページ遷移前の利用者stateを退避できるようにする。
 */
export function notifyAuthSessionReauthorizationStarted(): boolean {
  authRequestGeneration += 1;
  return notifyAuthSessionListeners("reauthorize");
}

function notifyAuthSessionListeners(event: AuthSessionLifecycleEvent): boolean {
  let ready = true;
  for (const listener of Array.from(authSessionEndListeners)) {
    try {
      if (listener(event) === false) ready = false;
    } catch {
      // One subscriber must not prevent other subscribers from seeing the event.
      ready = false;
    }
  }
  return ready;
}

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
