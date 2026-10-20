import {
  getToken as defaultGetToken,
  notifyAuthorized,
  notifyUnauthorized,
  type AuthorizedHandler,
  type TokenProvider,
  type UnauthorizedHandler,
} from "../auth";
import { joinUrl, type AppConfig } from "../config";

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

  constructor(status: number, message: string, body?: unknown, request?: ApiErrorRequest) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
    this.path = request?.path ?? null;
    this.method = request?.method ?? null;
  }
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
}

export interface ApiClient {
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

  async function buildHeaders(
    explicit: Record<string, string> | undefined,
    hasBody: boolean,
  ): Promise<Headers> {
    const headers = new Headers(explicit);
    const authHeaders = await buildAuthHeaders({ getToken: tokenProvider, devUserId });
    authHeaders.forEach((value, key) => headers.set(key, value));
    headers.set("Accept", "application/json");
    if (hasBody) {
      headers.set("Content-Type", "application/json");
    }

    return headers;
  }

  async function request<T>(path: string, requestOptions: RequestOptions = {}): Promise<T> {
    const hasBody = requestOptions.body !== undefined;
    const method = requestOptions.method ?? "GET";
    const headers = await buildHeaders(requestOptions.headers, hasBody);

    const response = await doFetch(joinUrl(baseUrl, path), {
      method,
      headers,
      body: hasBody ? JSON.stringify(requestOptions.body) : undefined,
      signal: requestOptions.signal,
    });

    const body = parseBody(await response.text());
    if (!response.ok) {
      if (response.status === 401) {
        onUnauthorized();
      }
      throw new ApiError(
        response.status,
        `Request to ${path} failed with status ${response.status}`,
        body,
        { path, method },
      );
    }
    onAuthorized();
    return body as T;
  }

  async function requestWithTx<T>(
    path: string,
    requestOptions: RequestOptions = {},
  ): Promise<ApiEnvelope<T>> {
    return unwrapEnvelope<T>(await request<unknown>(path, requestOptions));
  }

  return {
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
    ...options,
  });
}
