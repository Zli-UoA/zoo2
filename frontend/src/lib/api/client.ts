import { getToken as defaultGetToken, type TokenProvider } from "../auth";
import { joinUrl, type AppConfig } from "../config";

export interface ApiEnvelope<T> {
  data: T;
  txid: string;
}

export class ApiError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(status: number, message: string, body?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

export function isApiEnvelope(value: unknown): value is ApiEnvelope<unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    "data" in value &&
    "txid" in value &&
    typeof (value as { txid: unknown }).txid === "string"
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
  fetch?: FetchLike;
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
  const doFetch: FetchLike = options.fetch ?? ((input, init) => fetch(input, init));

  async function buildHeaders(
    explicit: Record<string, string> | undefined,
    hasBody: boolean,
  ): Promise<Headers> {
    const headers = new Headers(explicit);
    headers.set("Accept", "application/json");
    if (hasBody) {
      headers.set("Content-Type", "application/json");
    }

    const token = await tokenProvider();
    if (token) {
      headers.set("Authorization", `Bearer ${token}`);
    }
    if (devUserId) {
      headers.set("X-Dev-User", devUserId);
    }

    return headers;
  }

  async function request<T>(path: string, requestOptions: RequestOptions = {}): Promise<T> {
    const hasBody = requestOptions.body !== undefined;
    const headers = await buildHeaders(requestOptions.headers, hasBody);

    const response = await doFetch(joinUrl(baseUrl, path), {
      method: requestOptions.method ?? "GET",
      headers,
      body: hasBody ? JSON.stringify(requestOptions.body) : undefined,
      signal: requestOptions.signal,
    });

    const body = parseBody(await response.text());
    if (!response.ok) {
      throw new ApiError(
        response.status,
        `Request to ${path} failed with status ${response.status}`,
        body,
      );
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
