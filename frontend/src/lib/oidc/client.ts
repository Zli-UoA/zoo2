import { decodeJwtPayload, audienceIncludes, issuerMatches } from "./jwt";
import {
  type AuthStorage,
  clearLoggedOut,
  clearPending,
  markLoggedOut,
  savePending,
  takePending,
} from "./pending";
import { createChallenge, createNonce, createState, createVerifier } from "./pkce";

export type OidcFetch = (input: string, init?: RequestInit) => Promise<Response>;

/** 既定のコールバックパス。`VITE_OIDC_REDIRECT_URI` で差し替え可能。 */
export const DEFAULT_CALLBACK_PATH = "/auth/callback";

export const DEFAULT_OIDC_SCOPE = "openid profile email";

/** 期限切れ前に更新を始める余裕。ロングポーリング中の Shape もここで切れ目なく更新される。 */
const REFRESH_SKEW_MS = 60_000;

const DISCOVERY_TIMEOUT_MS = 10_000;
const TOKEN_TIMEOUT_MS = 10_000;

export class OidcError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "OidcError";
    this.code = code;
  }
}

export interface OidcEndpoints {
  issuer: string;
  authorizationEndpoint: string;
  tokenEndpoint: string;
  endSessionEndpoint: string | null;
}

export interface OidcTokens {
  accessToken: string;
  refreshToken: string | null;
  idToken: string | null;
  /** epoch ミリ秒。 */
  expiresAt: number;
}

export interface OidcEnvironment {
  origin: string;
  href: string;
  assign(url: string): void;
  replaceUrl(url: string): void;
  storage: AuthStorage;
  fetch: OidcFetch;
  now(): number;
}

export interface OidcClientOptions {
  issuer: string;
  clientId: string;
  redirectUri?: string | null;
  scope?: string;
  postLogoutRedirectUri?: string | null;
  environment?: OidcEnvironment;
}

/**
 * ブラウザ実装の実行環境。AuthGate の effect（ブラウザのみ）から生成する。
 * `window` を直接読むため、SSR / テストでは呼ばない。
 */
export function createBrowserEnvironment(): OidcEnvironment {
  return {
    get origin() {
      return window.location.origin;
    },
    get href() {
      return window.location.href;
    },
    assign: (url) => window.location.assign(url),
    replaceUrl: (url) => window.history.replaceState(null, "", url),
    storage: window.sessionStorage,
    fetch: (input, init) => fetch(input, init),
    now: () => Date.now(),
  };
}

const RETURN_TO_BASE = "http://localhost";

/**
 * 復帰先は同一オリジンの相対パスだけを許可する（オープンリダイレクト防止）。
 * ダミー origin で解決して origin / pathname を検証し、`//host` や
 * `/\host`（URL パーサはスキーム相対として扱う）も弾く。
 * コールバック自身に戻すとループするためルートへ倒す。
 */
export function sanitizeReturnTo(
  value: string | null | undefined,
  callbackPath: string = DEFAULT_CALLBACK_PATH,
): string {
  if (!value || !value.startsWith("/") || value.includes("\\")) {
    return "/";
  }
  let resolved: URL;
  try {
    resolved = new URL(value, RETURN_TO_BASE);
  } catch {
    return "/";
  }
  if (resolved.origin !== RETURN_TO_BASE) {
    return "/";
  }
  const normalize = (path: string) => (path.length > 1 ? path.replace(/\/+$/, "") : path);
  if (normalize(resolved.pathname) === normalize(callbackPath)) {
    return "/";
  }
  return `${resolved.pathname}${resolved.search}${resolved.hash}`;
}

interface TokenResponse {
  error?: unknown;
  error_description?: unknown;
  access_token?: unknown;
  refresh_token?: unknown;
  id_token?: unknown;
  expires_in?: unknown;
}

/**
 * OIDC Authorization Code + PKCE (S256) の最小クライアント。
 *
 * 依存を足さず、必要な範囲だけを実装する。署名検証は Rust の責務で、ここでは
 * state / nonce / iss / aud / exp による取り違え検知だけを行う。トークンは
 * メモリ保持のみ（sessionStorage には state / nonce / verifier だけを置く）。
 */
export class OidcClient {
  private readonly environment: OidcEnvironment;
  private readonly issuer: string;
  private readonly clientId: string;
  private readonly configuredRedirectUri: string | null;
  private readonly configuredPostLogoutRedirectUri: string | null;
  readonly scope: string;

  private endpoints: OidcEndpoints | null = null;
  private discoveryPromise: Promise<OidcEndpoints> | null = null;
  private tokens: OidcTokens | null = null;
  private refreshPromise: Promise<OidcTokens> | null = null;
  private callbackRequest: { url: string; promise: Promise<{ returnTo: string }> } | null = null;
  /** セッション破棄と進行中の refresh の競合を検知するための世代番号。 */
  private generation = 0;

  constructor(options: OidcClientOptions) {
    this.environment = options.environment ?? createBrowserEnvironment();
    this.issuer = options.issuer;
    this.clientId = options.clientId;
    this.configuredRedirectUri = options.redirectUri ?? null;
    this.configuredPostLogoutRedirectUri = options.postLogoutRedirectUri ?? null;
    this.scope = options.scope?.trim() || DEFAULT_OIDC_SCOPE;
  }

  get redirectUri(): string {
    return (
      this.configuredRedirectUri ??
      new URL(DEFAULT_CALLBACK_PATH, this.environment.origin).toString()
    );
  }

  get postLogoutRedirectUri(): string {
    return this.configuredPostLogoutRedirectUri ?? new URL("/", this.environment.origin).toString();
  }

  get hasSession(): boolean {
    return this.tokens !== null;
  }

  /** 現在の URL（または指定 URL）がコールバックかどうか。 */
  isCallbackUrl(url: string = this.environment.href): boolean {
    let current: URL;
    let callback: URL;
    try {
      current = new URL(url);
      callback = new URL(this.redirectUri);
    } catch {
      return false;
    }
    if (current.origin !== callback.origin || current.pathname !== callback.pathname) {
      return false;
    }
    return current.searchParams.has("code") || current.searchParams.has("error");
  }

  /** OpenID Discovery。結果はメモリにキャッシュし、失敗時は次回リトライさせる。 */
  async discover(): Promise<OidcEndpoints> {
    if (this.endpoints) {
      return this.endpoints;
    }
    this.discoveryPromise ??= this.fetchDiscovery().finally(() => {
      this.discoveryPromise = null;
    });
    return this.discoveryPromise;
  }

  private async fetchDiscovery(): Promise<OidcEndpoints> {
    const discoveryUrl = new URL(
      `${this.issuer.replace(/\/+$/, "")}/.well-known/openid-configuration`,
    ).toString();
    const response = await this.environment.fetch(discoveryUrl, {
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(DISCOVERY_TIMEOUT_MS),
    });
    if (!response.ok) {
      throw new OidcError(
        "discovery_failed",
        `OpenID Discovery が ${response.status} を返しました`,
      );
    }
    const body = (await response.json()) as Record<string, unknown>;
    if (!issuerMatches(typeof body.issuer === "string" ? body.issuer : undefined, this.issuer)) {
      throw new OidcError("issuer_mismatch", "Discovery の issuer が設定と一致しません");
    }
    const authorizationEndpoint = body.authorization_endpoint;
    const tokenEndpoint = body.token_endpoint;
    if (typeof authorizationEndpoint !== "string" || typeof tokenEndpoint !== "string") {
      throw new OidcError("invalid_discovery", "Discovery に必須エンドポイントがありません");
    }
    const endpoints: OidcEndpoints = {
      issuer: String(body.issuer),
      authorizationEndpoint,
      tokenEndpoint,
      endSessionEndpoint:
        typeof body.end_session_endpoint === "string" ? body.end_session_endpoint : null,
    };
    this.endpoints = endpoints;
    return endpoints;
  }

  /** authorize エンドポイントへリダイレクトする。戻り値はテスト用の URL。 */
  async login(returnTo?: string): Promise<string> {
    const endpoints = await this.discover();
    const pending = {
      state: createState(),
      nonce: createNonce(),
      verifier: createVerifier(),
      returnTo: sanitizeReturnTo(returnTo, new URL(this.redirectUri).pathname),
      createdAt: this.environment.now(),
    };
    const challenge = await createChallenge(pending.verifier);
    savePending(this.environment.storage, pending);

    const url = new URL(endpoints.authorizationEndpoint);
    url.searchParams.set("response_type", "code");
    url.searchParams.set("client_id", this.clientId);
    url.searchParams.set("redirect_uri", this.redirectUri);
    url.searchParams.set("scope", this.scope);
    url.searchParams.set("state", pending.state);
    url.searchParams.set("nonce", pending.nonce);
    url.searchParams.set("code_challenge", challenge);
    url.searchParams.set("code_challenge_method", "S256");

    const href = url.toString();
    this.environment.assign(href);
    return href;
  }

  /**
   * コールバックを処理する。成功時はトークンをメモリに保持し復帰先を返す。
   * URL の書き換え（履歴からの除去）は呼び出し側が行う。
   *
   * pending は使い捨てなので、同じコールバック URL の再処理は進行中の結果を
   * 共有する（React StrictMode の effect 二重実行や再マウントで state を
   * 二重消費しないため）。
   */
  handleCallback(url: string = this.environment.href): Promise<{ returnTo: string }> {
    if (this.callbackRequest?.url === url) {
      return this.callbackRequest.promise;
    }
    const promise = this.performCallback(url);
    this.callbackRequest = { url, promise };
    return promise;
  }

  private async performCallback(url: string): Promise<{ returnTo: string }> {
    const current = new URL(url);
    const params = current.searchParams;
    const error = params.get("error");
    if (error) {
      clearPending(this.environment.storage);
      throw new OidcError(error, params.get("error_description") ?? `認可エラー: ${error}`);
    }
    const code = params.get("code");
    const state = params.get("state");
    if (!code || !state) {
      throw new OidcError("invalid_callback", "コールバックに code / state がありません");
    }
    const pending = takePending(this.environment.storage, this.environment.now());
    if (!pending) {
      throw new OidcError(
        "missing_pending_request",
        "保存された認可リクエストが無いため state / nonce を検証できません",
      );
    }
    if (pending.state !== state) {
      throw new OidcError("state_mismatch", "state が一致しません");
    }

    const body = new URLSearchParams({
      grant_type: "authorization_code",
      code,
      redirect_uri: this.redirectUri,
      client_id: this.clientId,
      code_verifier: pending.verifier,
    });
    const responseBody = await this.requestToken(body);
    // nonce を検証できるよう、コード交換では ID トークンを必須にする。
    this.tokens = this.parseTokens(responseBody, pending.nonce, {
      refreshToken: null,
      idToken: null,
    });
    clearLoggedOut(this.environment.storage);
    return { returnTo: sanitizeReturnTo(pending.returnTo, new URL(this.redirectUri).pathname) };
  }

  /**
   * 有効なアクセストークンを返す。期限が近ければ単一の refresh で更新し、
   * 更新できない場合はキャッシュを捨てて null を返す（呼び出し側の 401 で再ログイン）。
   */
  async getAccessToken(): Promise<string | null> {
    const tokens = this.tokens;
    if (!tokens) {
      return null;
    }
    if (tokens.expiresAt - this.environment.now() > REFRESH_SKEW_MS) {
      return tokens.accessToken;
    }
    if (!tokens.refreshToken) {
      this.tokens = null;
      return null;
    }
    try {
      return (await this.refreshTokens()).accessToken;
    } catch {
      this.tokens = null;
      return null;
    }
  }

  resetSession(): void {
    this.generation += 1;
    this.tokens = null;
  }

  /**
   * refresh_token でアクセストークンを更新する。同時呼び出しは 1 回のリクエストに
   * 集約する（Kanidm は refresh の再利用を検知するとセッションを失効させるため）。
   */
  private refreshTokens(): Promise<OidcTokens> {
    this.refreshPromise ??= this.performRefresh().finally(() => {
      this.refreshPromise = null;
    });
    return this.refreshPromise;
  }

  private async performRefresh(): Promise<OidcTokens> {
    const current = this.tokens;
    const refreshToken = current?.refreshToken;
    if (!refreshToken) {
      throw new OidcError("no_refresh_token", "refresh_token がありません");
    }
    const generation = this.generation;
    const body = new URLSearchParams({
      grant_type: "refresh_token",
      refresh_token: refreshToken,
      client_id: this.clientId,
    });
    const responseBody = await this.requestToken(body);
    // refresh 応答に id_token が無い IdP でも、logout の id_token_hint 用に
    // 前回の値を引き継ぐ。
    const tokens = this.parseTokens(responseBody, null, {
      refreshToken,
      idToken: current?.idToken ?? null,
    });
    if (this.generation !== generation) {
      // 更新中にログアウト等でセッションが破棄された。新しいトークンは捨てる。
      throw new OidcError("session_reset", "セッションが破棄されました");
    }
    this.tokens = tokens;
    return tokens;
  }

  /**
   * ローカルのトークンを破棄し、IdP が RP-initiated logout に対応していれば
   * end_session_endpoint へ、対応していなければログアウト画面へ遷移する。
   *
   * Kanidm は end_session_endpoint を公開していないため、実質ローカルログアウトに
   * なる。Kanidm のセッション自体を終えるには Kanidm 側でログアウトする必要がある。
   */
  async logout(): Promise<void> {
    const tokens = this.tokens;
    this.generation += 1;
    this.tokens = null;
    this.refreshPromise = null;
    clearPending(this.environment.storage);
    markLoggedOut(this.environment.storage);

    const endpoints = await this.discover().catch(() => null);
    if (endpoints?.endSessionEndpoint) {
      const url = new URL(endpoints.endSessionEndpoint);
      url.searchParams.set("client_id", this.clientId);
      url.searchParams.set("post_logout_redirect_uri", this.postLogoutRedirectUri);
      if (tokens?.idToken) {
        url.searchParams.set("id_token_hint", tokens.idToken);
      }
      this.environment.assign(url.toString());
      return;
    }
    this.environment.assign(this.postLogoutRedirectUri);
  }

  private async requestToken(body: URLSearchParams): Promise<TokenResponse> {
    const endpoints = await this.discover();
    const response = await this.environment.fetch(endpoints.tokenEndpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded",
        Accept: "application/json",
      },
      body: body.toString(),
      signal: AbortSignal.timeout(TOKEN_TIMEOUT_MS),
    });
    let parsed: unknown;
    try {
      parsed = await response.json();
    } catch {
      throw new OidcError("invalid_token_response", "トークン応答が JSON ではありません");
    }
    if (typeof parsed !== "object" || parsed === null) {
      throw new OidcError("invalid_token_response", "トークン応答の形式が不正です");
    }
    const parsedBody = parsed as TokenResponse;
    if (!response.ok) {
      const code = typeof parsedBody.error === "string" ? parsedBody.error : "token_request_failed";
      const description =
        typeof parsedBody.error_description === "string"
          ? parsedBody.error_description
          : `トークンエンドポイントが ${response.status} を返しました`;
      throw new OidcError(code, description);
    }
    return parsedBody;
  }

  private parseTokens(
    body: TokenResponse,
    expectedNonce: string | null,
    previous: { refreshToken: string | null; idToken: string | null },
  ): OidcTokens {
    const accessToken = body.access_token;
    if (typeof accessToken !== "string" || accessToken.length === 0) {
      throw new OidcError("invalid_token_response", "access_token がありません");
    }
    const idToken = typeof body.id_token === "string" ? body.id_token : null;
    if (!idToken && expectedNonce !== null) {
      // nonce を要求したコード交換で ID トークンが無い応答は受け付けない。
      throw new OidcError("invalid_id_token", "id_token がありません");
    }
    if (idToken) {
      this.validateIdToken(idToken, expectedNonce);
    }
    const expiresIn =
      typeof body.expires_in === "number" && Number.isFinite(body.expires_in) && body.expires_in > 0
        ? body.expires_in
        : 300;
    return {
      accessToken,
      refreshToken:
        typeof body.refresh_token === "string" ? body.refresh_token : previous.refreshToken,
      idToken: idToken ?? previous.idToken,
      expiresAt: this.environment.now() + expiresIn * 1000,
    };
  }

  private validateIdToken(idToken: string, expectedNonce: string | null): void {
    const payload = decodeJwtPayload(idToken);
    if (!payload) {
      throw new OidcError("invalid_id_token", "ID トークンをデコードできません");
    }
    if (!issuerMatches(payload.iss, this.issuer)) {
      throw new OidcError("id_token_issuer_mismatch", "ID トークンの iss が一致しません");
    }
    if (!audienceIncludes(payload.aud, this.clientId)) {
      throw new OidcError("id_token_audience_mismatch", "ID トークンの aud が一致しません");
    }
    if (typeof payload.exp !== "number" || payload.exp * 1000 <= this.environment.now() - 60_000) {
      throw new OidcError("id_token_expired", "ID トークンが期限切れです");
    }
    if (expectedNonce !== null && payload.nonce !== expectedNonce) {
      throw new OidcError("nonce_mismatch", "ID トークンの nonce が一致しません");
    }
  }
}
