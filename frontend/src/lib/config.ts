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

export interface AppConfig {
  shapeMode: ShapeMode;
  txidMode: TxidMode;
  electricShapeUrl: string;
  apiBaseUrl: string;
  shapeProxyPath: string;
  devUserId: string | null;
}

export const DEFAULT_CONFIG: AppConfig = {
  shapeMode: "proxy",
  txidMode: "awaitTxId",
  electricShapeUrl: "http://localhost:3000/v1/shape",
  apiBaseUrl: "http://localhost:8000",
  shapeProxyPath: "/api/shapes",
  devUserId: null,
};

export interface ConfigEnv {
  DEV?: boolean;
  VITE_SHAPE_MODE?: string;
  VITE_TXID_MODE?: string;
  VITE_ELECTRIC_SHAPE_URL?: string;
  VITE_API_BASE_URL?: string;
  VITE_SHAPE_PROXY_PATH?: string;
  VITE_DEV_USER_ID?: string;
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
 */
function resolveDevUserId(
  mode: ShapeMode,
  isDev: boolean,
  configured: string | undefined,
): string | null {
  if (!isDev) {
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
  return {
    ...DEFAULT_CONFIG,
    shapeMode,
    txidMode: resolveTxidMode(env.VITE_TXID_MODE),
    electricShapeUrl: env.VITE_ELECTRIC_SHAPE_URL ?? DEFAULT_CONFIG.electricShapeUrl,
    apiBaseUrl: env.VITE_API_BASE_URL ?? DEFAULT_CONFIG.apiBaseUrl,
    shapeProxyPath: env.VITE_SHAPE_PROXY_PATH ?? DEFAULT_CONFIG.shapeProxyPath,
    devUserId: resolveDevUserId(shapeMode, isDev, env.VITE_DEV_USER_ID),
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
