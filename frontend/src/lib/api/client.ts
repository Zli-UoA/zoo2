import {
  getToken as defaultGetToken,
  getAuthRequestGeneration,
  getAuthSessionGeneration,
  getTokenProviderGeneration,
  notifyAuthorized,
  notifyUnauthorized,
  type AuthorizedHandler,
  type TokenProvider,
  type UnauthorizedHandler,
} from "../auth";
import { joinUrl, type AppConfig } from "../config";
import type { ChannelNotificationLevel } from "../electric/types";
import type {
  ChannelNotificationSettings,
  ChannelPinsResponse,
  PinChannelResult,
  ReorderChannelPinsResult,
  UnpinChannelResult,
} from "./types";
import { apiPath, assertSafeApiPath } from "./path";
import { normalizeRouteGroup, reportApiError } from "../telemetry/capture";
import { runWithHttpSpan } from "../telemetry/tracer";

export interface ApiEnvelope<T> {
  data: T;
  /** null means an idempotent no-op without a replicated database write. */
  txid: string | null;
}

export interface ApiErrorRequest {
  path: string;
  method: string;
}

export class ApiError extends Error {
  readonly status: number;
  readonly body: unknown;
  /** 失敗したリクエストのパス。呼び出し側が操作に応じた文言を選ぶために使う。 */
  readonly path: string | null;
  /** 失敗したリクエストの HTTP メソッド。 */
  readonly method: string | null;
  /** 応答の `Retry-After` ヘッダー（秒）。読めない場合は `null`。 */
  readonly retryAfter: number | null;

  constructor(
    status: number,
    message: string,
    body?: unknown,
    request?: ApiErrorRequest,
    retryAfter: number | null = null,
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
    this.path = request?.path ?? null;
    this.method = request?.method ?? null;
    this.retryAfter = retryAfter;
  }
}

/** `Retry-After` は秒数（推奨）と HTTP 日時の両方で来るので、秒数に正規化する。 */
function parseRetryAfter(value: string | null): number | null {
  if (value === null) return null;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return seconds;
  const date = Date.parse(value);
  if (!Number.isNaN(date)) return Math.max(0, Math.ceil((date - Date.now()) / 1000));
  return null;
}

export function isApiEnvelope(value: unknown): value is ApiEnvelope<unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    "data" in value &&
    "txid" in value &&
    (value.txid === null || typeof value.txid === "string")
  );
}

export function unwrapEnvelope<T>(body: unknown): ApiEnvelope<T> {
  if (!isApiEnvelope(body)) {
    throw new Error("Expected API response envelope { data, txid }");
  }
  return body as ApiEnvelope<T>;
}

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export interface ApiClientOptions {
  baseUrl?: string;
  devUserId?: string | null;
  getToken?: TokenProvider;
  /** 401 時の再ログイン導線。既定は `auth.notifyUnauthorized`。 */
  onUnauthorized?: UnauthorizedHandler;
  /** 成功時の通知（再ログイン試行カウンタの消去用）。既定は `auth.notifyAuthorized`。 */
  onAuthorized?: AuthorizedHandler;
  fetch?: FetchLike;
  /** OpenTelemetry traceparent 注入と 5xx/通信エラー計測を有効にするか（既定 true）。 */
  telemetry?: boolean;
}

export interface AuthHeadersSource {
  getToken?: TokenProvider;
  devUserId?: string | null;
}

export async function buildAuthHeaders(source: AuthHeadersSource = {}): Promise<Headers> {
  const getToken = source.getToken ?? defaultGetToken;
  const headers = new Headers();
  const token = await getToken();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }
  if (source.devUserId) {
    headers.set("X-Dev-User", source.devUserId);
  }
  return headers;
}

export interface RequestOptions {
  method?: string;
  body?: unknown;
  headers?: Record<string, string>;
  signal?: AbortSignal;
  keepalive?: boolean;
}

/** Start the request synchronously while pagehide still owns the document. */
export function sendKeepaliveRequest(
  url: string,
  body: unknown,
  auth: HeadersInit,
  doFetch: FetchLike = fetch,
): Promise<void> {
  const headers = new Headers(auth);
  headers.set("Content-Type", "application/json");
  headers.set("Accept", "application/json");
  return doFetch(url, {
    method: "POST",
    headers,
    keepalive: true,
    body: JSON.stringify(body),
  }).then(() => undefined);
}

export interface ApiClient {
  /** Uses this client's most recently resolved auth headers without asynchronous token refresh. */
  keepalive?(path: string, body: unknown): Promise<void>;
  request<T>(path: string, options?: RequestOptions): Promise<T>;
  requestWithTx<T>(path: string, options?: RequestOptions): Promise<ApiEnvelope<T>>;
  get<T>(path: string, options?: Omit<RequestOptions, "method" | "body">): Promise<T>;
  post<T>(path: string, body?: unknown, options?: RequestOptions): Promise<ApiEnvelope<T>>;
  patch<T>(path: string, body?: unknown, options?: RequestOptions): Promise<ApiEnvelope<T>>;
  delete<T>(path: string, options?: RequestOptions): Promise<ApiEnvelope<T>>;
}

function parseBody(text: string): unknown {
  if (!text) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

export function createApiClient(options: ApiClientOptions = {}): ApiClient {
  const baseUrl = options.baseUrl ?? "";
  const devUserId = options.devUserId ?? null;
  const tokenProvider = options.getToken ?? defaultGetToken;
  const onUnauthorized = options.onUnauthorized ?? notifyUnauthorized;
  const onAuthorized = options.onAuthorized ?? notifyAuthorized;
  const doFetch: FetchLike = options.fetch ?? ((input, init) => fetch(input, init));
  const telemetry = options.telemetry ?? true;

  let lastAuthHeaders: Headers | undefined;

  async function buildHeaders(
    explicit: Record<string, string> | undefined,
    hasBody: boolean,
  ): Promise<{ headers: Headers; authHeaders: Headers }> {
    const headers = new Headers(explicit);
    const authHeaders = await buildAuthHeaders({ getToken: tokenProvider, devUserId });
    authHeaders.forEach((value, key) => headers.set(key, value));
    headers.set("Accept", "application/json");
    if (hasBody) {
      headers.set("Content-Type", "application/json");
    }

    return { headers, authHeaders };
  }

  function assertRequestStillValid(
    signal: AbortSignal | undefined,
    expectedAuthSessionGen: number,
    expectedTokenProviderGen: number,
  ): void {
    if (signal?.aborted) {
      throw signal.reason ?? new DOMException("The operation was aborted.", "AbortError");
    }
    if (
      expectedAuthSessionGen !== getAuthSessionGeneration() ||
      (!options.getToken && expectedTokenProviderGen !== getTokenProviderGeneration())
    ) {
      throw new DOMException("The operation was aborted.", "AbortError");
    }
  }

  async function request<T>(path: string, requestOptions: RequestOptions = {}): Promise<T> {
    const authSessionGeneration = getAuthSessionGeneration();
    const authRequestGeneration = getAuthRequestGeneration();
    const tokenProviderGeneration = getTokenProviderGeneration();
    assertSafeApiPath(path);
    assertRequestStillValid(requestOptions.signal, authSessionGeneration, tokenProviderGeneration);
    const hasBody = requestOptions.body !== undefined;
    const method = requestOptions.method ?? "GET";
    const requestRoute = telemetry ? normalizeRouteGroup() : undefined;
    const { headers, authHeaders } = await buildHeaders(requestOptions.headers, hasBody);
    assertRequestStillValid(requestOptions.signal, authSessionGeneration, tokenProviderGeneration);
    lastAuthHeaders = new Headers(authHeaders);
    const url = joinUrl(baseUrl, path);
    const startedAt = Date.now();

    let responseStatus = 0;
    let errorReported = false;
    const { res: response, rawText } = await runWithHttpSpan(
      { enabled: telemetry, method, url, apiBaseUrl: baseUrl, headers },
      async (traceIds, spanControl) => {
        try {
          const res = await doFetch(url, {
            method,
            headers,
            body: hasBody ? JSON.stringify(requestOptions.body) : undefined,
            signal: requestOptions.signal,
            keepalive: requestOptions.keepalive,
          });
          responseStatus = res.status;
          spanControl.setResponseStatus(res.status);
          if (telemetry && res.status >= 500) {
            errorReported = true;
            reportApiError({
              httpStatus: res.status,
              durationMs: Date.now() - startedAt,
              error: new Error(`HTTP ${res.status}`),
              route: requestRoute,
              traceIds,
            });
          }
          const rawText = await res.text();
          return { res, rawText };
        } catch (error) {
          const isAbort = error instanceof DOMException && error.name === "AbortError";
          if (telemetry && !isAbort && !errorReported) {
            errorReported = true;
            reportApiError({
              httpStatus: responseStatus >= 500 ? responseStatus : 0,
              durationMs: Date.now() - startedAt,
              error,
              route: requestRoute,
              traceIds,
            });
          }
          throw error;
        }
      },
    );

    const body = parseBody(rawText);
    if (!response.ok) {
      if (
        response.status === 401 &&
        authSessionGeneration === getAuthSessionGeneration() &&
        authRequestGeneration === getAuthRequestGeneration()
      ) {
        onUnauthorized();
      }
      throw new ApiError(
        response.status,
        `Request to ${path} failed with status ${response.status}`,
        body,
        { path, method },
        parseRetryAfter(response.headers.get("Retry-After")),
      );
    }
    if (
      authSessionGeneration === getAuthSessionGeneration() &&
      authRequestGeneration === getAuthRequestGeneration()
    ) {
      onAuthorized();
    }
    return body as T;
  }

  async function requestWithTx<T>(
    path: string,
    requestOptions: RequestOptions = {},
  ): Promise<ApiEnvelope<T>> {
    return unwrapEnvelope<T>(await request<unknown>(path, requestOptions));
  }

  return {
    keepalive: (path, body) =>
      lastAuthHeaders
        ? sendKeepaliveRequest(joinUrl(baseUrl, path), body, lastAuthHeaders, doFetch)
        : Promise.reject(new Error("No resolved authentication for keepalive request")),
    request,
    requestWithTx,
    get: (path, requestOptions) => request(path, { ...requestOptions, method: "GET" }),
    post: (path, body, requestOptions) =>
      requestWithTx(path, { ...requestOptions, method: "POST", body }),
    patch: (path, body, requestOptions) =>
      requestWithTx(path, { ...requestOptions, method: "PATCH", body }),
    delete: (path, requestOptions) => requestWithTx(path, { ...requestOptions, method: "DELETE" }),
  };
}

export function createConfiguredApiClient(
  config: AppConfig,
  options: Omit<ApiClientOptions, "baseUrl" | "devUserId"> = {},
): ApiClient {
  return createApiClient({
    baseUrl: config.apiBaseUrl,
    devUserId: config.devUserId,
    telemetry: config.telemetryEnabled,
    ...options,
  });
}

/**
 * ユーザーごとのチャンネル・DM のピン留め API。
 *
 * 読み取りは `GET /api/channel-pins`、書き込みは `POST` / `DELETE` /
 * `PATCH /api/channel-pins/order`。競合したときの 409 応答は `ApiError.body` に
 * `{ error, code: "CHANNEL_PINS_CONFLICT", version, pins }` を持つ。
 */
export function createChannelPinsApi(client: ApiClient) {
  return {
    getChannelPins: () => client.get<ChannelPinsResponse>("/api/channel-pins"),
    pinChannel: (channelId: string, expectedVersion?: number) =>
      client.post<PinChannelResult>("/api/channel-pins", {
        channel_id: channelId,
        ...(expectedVersion === undefined ? {} : { expected_version: expectedVersion }),
      }),
    unpinChannel: (channelId: string, expectedVersion?: number) =>
      client.delete<UnpinChannelResult>(
        `/api/channel-pins/${encodeURIComponent(channelId)}${
          expectedVersion === undefined
            ? ""
            : `?${new URLSearchParams({ expected_version: String(expectedVersion) })}`
        }`,
      ),
    reorderChannelPins: (channelIds: string[], expectedVersion: number) =>
      client.patch<ReorderChannelPinsResult>("/api/channel-pins/order", {
        channel_ids: channelIds,
        expected_version: expectedVersion,
      }),
  };
}

/**
 * チャンネルごとの通知レベル。
 *
 * 表示値は `channelNotificationSettings` コレクションから読むため、画面ごとに GET はしない。
 * GET はコレクションにない場合の個別確認と API テスト用に置く。
 */
export function createChannelNotificationSettingsApi(client: ApiClient) {
  return {
    getChannelNotificationSettings: (channelId: string) =>
      client.get<ChannelNotificationSettings>(
        apiPath`/api/channels/${channelId}/notification-settings`,
      ),
    updateChannelNotificationSettings: (channelId: string, level: ChannelNotificationLevel) =>
      client.patch<ChannelNotificationSettings>(
        apiPath`/api/channels/${channelId}/notification-settings`,
        { level },
      ),
  };
}
