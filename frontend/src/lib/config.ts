import { SHAPE_META, type ShapeName } from "./electric/schema";

export type ShapeMode = "direct" | "proxy";

export interface AppConfig {
  shapeMode: ShapeMode;
  electricShapeUrl: string;
  apiBaseUrl: string;
  shapeProxyPath: string;
  devUserId: string | null;
}

export const DEFAULT_CONFIG: AppConfig = {
  shapeMode: "direct",
  electricShapeUrl: "http://localhost:3000/v1/shape",
  apiBaseUrl: "http://localhost:8000",
  shapeProxyPath: "/api/shapes",
  devUserId: null,
};

export type ConfigEnv = Partial<
  Record<
    | "VITE_SHAPE_MODE"
    | "VITE_ELECTRIC_SHAPE_URL"
    | "VITE_API_BASE_URL"
    | "VITE_SHAPE_PROXY_PATH"
    | "VITE_DEV_USER_ID",
    string | undefined
  >
>;

function readEnv(): ConfigEnv {
  const env = (import.meta.env ?? {}) as Record<string, string | undefined>;
  return {
    VITE_SHAPE_MODE: env.VITE_SHAPE_MODE,
    VITE_ELECTRIC_SHAPE_URL: env.VITE_ELECTRIC_SHAPE_URL,
    VITE_API_BASE_URL: env.VITE_API_BASE_URL,
    VITE_SHAPE_PROXY_PATH: env.VITE_SHAPE_PROXY_PATH,
    VITE_DEV_USER_ID: env.VITE_DEV_USER_ID,
  };
}

export function resolveConfig(
  env: ConfigEnv = readEnv(),
  overrides: Partial<AppConfig> = {},
): AppConfig {
  return {
    ...DEFAULT_CONFIG,
    shapeMode: env.VITE_SHAPE_MODE === "proxy" ? "proxy" : "direct",
    electricShapeUrl: env.VITE_ELECTRIC_SHAPE_URL ?? DEFAULT_CONFIG.electricShapeUrl,
    apiBaseUrl: env.VITE_API_BASE_URL ?? DEFAULT_CONFIG.apiBaseUrl,
    shapeProxyPath: env.VITE_SHAPE_PROXY_PATH ?? DEFAULT_CONFIG.shapeProxyPath,
    devUserId: env.VITE_DEV_USER_ID ?? null,
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
