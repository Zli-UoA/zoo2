import { SHAPE_META, type ShapeName } from "./electric/schema";

export type ShapeMode = "direct" | "proxy";

/**
 * 書き込み後に楽観状態を Electric のストリームと突き合わせる方法。
 *
 * - `awaitTxId`: REST レスポンスの `txid` を Electric が配信するまで待つ。
 * - `awaitMatch`: `txid` を返さない環境向け。operation と行 id が一致する
 *   change message が届くまで待つ。
 * - `skip`: 待機せず即座に解決する（ハンドシェイク未対応環境の暫定）。
 */
export type TxidMode = "awaitTxId" | "awaitMatch" | "skip";

export const TXID_MODES: readonly TxidMode[] = ["awaitTxId", "awaitMatch", "skip"];

/**
 * SPA が OIDC PKCE で利用する設定。未設定なら従来どおり dev（X-Dev-User）で動く。
 * Kanidm の確定値は #50 で差し替えられるよう、すべて環境変数で注入する。
 */
export interface OidcSettings {
  issuer: string;
  clientId: string;
  /** 未設定なら `${origin}/auth/callback` を実行時に組み立てる。 */
  redirectUri: string | null;
  scope: string;
  postLogoutRedirectUri: string | null;
}

export const DEFAULT_OIDC_SCOPE = "openid profile";

export interface AppConfig {
  shapeMode: ShapeMode;
  txidMode: TxidMode;
  electricShapeUrl: string;
  apiBaseUrl: string;
  shapeProxyPath: string;
  devUserId: string | null;
  oidc: OidcSettings | null;
}

export const DEFAULT_CONFIG: AppConfig = {
  shapeMode: "proxy",
  txidMode: "awaitTxId",
  electricShapeUrl: "http://localhost:3000/v1/shape",
  apiBaseUrl: "http://localhost:8000",
  shapeProxyPath: "/api/shapes",
  devUserId: null,
  oidc: null,
};

export interface ConfigEnv {
  DEV?: boolean;
  VITE_SHAPE_MODE?: string;
  VITE_TXID_MODE?: string;
  VITE_ELECTRIC_SHAPE_URL?: string;
  VITE_API_BASE_URL?: string;
  VITE_SHAPE_PROXY_PATH?: string;
  VITE_DEV_USER_ID?: string;
  VITE_OIDC_ISSUER?: string;
  VITE_OIDC_CLIENT_ID?: string;
  VITE_OIDC_REDIRECT_URI?: string;
  VITE_OIDC_SCOPE?: string;
  VITE_OIDC_POST_LOGOUT_REDIRECT_URI?: string;
}

function readEnv(): ConfigEnv {
  return (import.meta.env ?? {}) as ConfigEnv;
}

/**
 * Shape モードを決める。既定は常に `proxy`（バックエンドの Shape プロキシ経由）。
 * `direct`（Electric 直結）は明示したときだけ使う、認可を迂回する開発専用の経路。
 */
function resolveShapeMode(configured: string | undefined): ShapeMode {
  if (configured === "direct" || configured === "proxy") {
    return configured;
  }
  if (configured) {
    throw new Error(`Invalid VITE_SHAPE_MODE "${configured}": expected "direct" or "proxy"`);
  }
  return "proxy";
}

/**
 * 開発時の現在ユーザー（X-Dev-User）を決める。
 *
 * proxy モードはバックエンドが `X-Dev-User` でユーザーを識別するため、開発時は
 * `VITE_DEV_USER_ID` を必須にする。未設定だと現在ユーザーが分からず、参加/退出や
 * 自分のメッセージの編集・削除ができない（Phase 4 からの申し送り）。direct は
 * 認可なしの開発専用経路なので未設定を許す。本番ビルドでは常に null（トークンを使う）。
 *
 * OIDC を設定した場合は Bearer で認証するため `X-Dev-User` は送らない。dev でも
 * `VITE_DEV_USER_ID` を要求しない（staging の IdP に繋いで確認するため）。
 */
function resolveDevUserId(
  mode: ShapeMode,
  isDev: boolean,
  configured: string | undefined,
  oidcConfigured: boolean,
): string | null {
  if (!isDev || oidcConfigured) {
    return null;
  }
  if (configured) {
    return configured;
  }
  if (mode === "proxy") {
    throw new Error(
      "VITE_DEV_USER_ID is required in development with the Shape proxy (VITE_SHAPE_MODE=proxy, the default); it is sent as the X-Dev-User header",
    );
  }
  return null;
}

/**
 * OIDC 設定を環境変数から解決する。issuer と client ID は必ず対で設定する。
 * どちらも未設定なら null（従来の dev 動作）。片方だけの設定はタイポを黙って
 * 無視しないようエラーにする。
 */
function resolveOidc(env: ConfigEnv): OidcSettings | null {
  const issuer = env.VITE_OIDC_ISSUER?.trim();
  const clientId = env.VITE_OIDC_CLIENT_ID?.trim();
  if (!issuer && !clientId) {
    return null;
  }
  if (!issuer || !clientId) {
    throw new Error(
      "VITE_OIDC_ISSUER and VITE_OIDC_CLIENT_ID must be set together to enable OIDC login",
    );
  }
  return {
    issuer,
    clientId,
    redirectUri: env.VITE_OIDC_REDIRECT_URI?.trim() || null,
    scope: env.VITE_OIDC_SCOPE?.trim() || DEFAULT_OIDC_SCOPE,
    postLogoutRedirectUri: env.VITE_OIDC_POST_LOGOUT_REDIRECT_URI?.trim() || null,
  };
}

export function resolveTxidMode(configured: string | undefined): TxidMode {
  if (!configured) {
    return DEFAULT_CONFIG.txidMode;
  }
  if (TXID_MODES.includes(configured as TxidMode)) {
    return configured as TxidMode;
  }
  throw new Error(
    `Invalid VITE_TXID_MODE "${configured}": expected "awaitTxId", "awaitMatch", or "skip"`,
  );
}

export function resolveConfig(
  env: ConfigEnv = readEnv(),
  overrides: Partial<AppConfig> = {},
): AppConfig {
  const isDev = env.DEV === true;
  const shapeMode = resolveShapeMode(env.VITE_SHAPE_MODE);
  const oidc = resolveOidc(env);
  return {
    ...DEFAULT_CONFIG,
    shapeMode,
    txidMode: resolveTxidMode(env.VITE_TXID_MODE),
    electricShapeUrl: env.VITE_ELECTRIC_SHAPE_URL ?? DEFAULT_CONFIG.electricShapeUrl,
    apiBaseUrl: env.VITE_API_BASE_URL ?? DEFAULT_CONFIG.apiBaseUrl,
    shapeProxyPath: env.VITE_SHAPE_PROXY_PATH ?? DEFAULT_CONFIG.shapeProxyPath,
    devUserId: resolveDevUserId(shapeMode, isDev, env.VITE_DEV_USER_ID, oidc !== null),
    oidc,
    ...overrides,
  };
}

export function joinUrl(base: string, path: string): string {
  if (!base) {
    return path;
  }
  return `${base.replace(/\/+$/, "")}/${path.replace(/^\/+/, "")}`;
}

function withParams(url: string, params: Record<string, string | number | undefined>): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) {
      query.set(key, String(value));
    }
  }
  const serialized = query.toString();
  if (!serialized) {
    return url;
  }
  return url.includes("?") ? `${url}&${serialized}` : `${url}?${serialized}`;
}

export function buildShapeUrl(
  shape: ShapeName,
  config: AppConfig,
  params: Record<string, string | number | undefined> = {},
): string {
  const meta = SHAPE_META[shape];
  if (config.shapeMode === "direct") {
    return withParams(config.electricShapeUrl, { table: meta.table, ...params });
  }
  return withParams(
    joinUrl(config.apiBaseUrl, `${config.shapeProxyPath}/${meta.proxyPath}`),
    params,
  );
}
