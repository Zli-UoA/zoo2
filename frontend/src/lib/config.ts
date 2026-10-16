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

function resolveShapeMode(configured: string | undefined, isDev: boolean): ShapeMode {
  if (configured === "direct" || configured === "proxy") {
    return configured;
  }
  if (configured) {
    throw new Error(`Invalid VITE_SHAPE_MODE "${configured}": expected "direct" or "proxy"`);
  }
  return isDev ? "direct" : "proxy";
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
  return {
    ...DEFAULT_CONFIG,
    shapeMode: resolveShapeMode(env.VITE_SHAPE_MODE, isDev),
    txidMode: resolveTxidMode(env.VITE_TXID_MODE),
    electricShapeUrl: env.VITE_ELECTRIC_SHAPE_URL ?? DEFAULT_CONFIG.electricShapeUrl,
    apiBaseUrl: env.VITE_API_BASE_URL ?? DEFAULT_CONFIG.apiBaseUrl,
    shapeProxyPath: env.VITE_SHAPE_PROXY_PATH ?? DEFAULT_CONFIG.shapeProxyPath,
    devUserId: isDev ? (env.VITE_DEV_USER_ID ?? null) : null,
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
