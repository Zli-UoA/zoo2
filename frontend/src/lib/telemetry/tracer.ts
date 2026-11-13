import {
  context,
  propagation,
  trace,
  SpanKind,
  SpanStatusCode,
  type SpanContext,
  type TextMapSetter,
} from "@opentelemetry/api";
import { W3CTraceContextPropagator } from "@opentelemetry/core";
import { type WebTracerConfig, WebTracerProvider } from "@opentelemetry/sdk-trace-web";

const TRACER_NAME = "zoo-frontend";
const FALLBACK_ORIGIN = "http://localhost";

let initializedProvider: WebTracerProvider | null = null;
let propagatorRegistered = false;

const headerSetter: TextMapSetter<Headers> = {
  set(carrier, key, value) {
    carrier.set(key, value);
  },
};

function currentOrigin(explicitOrigin?: string): string {
  if (explicitOrigin) {
    return explicitOrigin;
  }
  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin;
  }
  return FALLBACK_ORIGIN;
}

/**
 * ブラウザ用 OpenTelemetry WebTracerProvider と W3C TraceContextPropagator を初期化する。
 * SSR 環境や無効設定時は安全にスキップする。
 */
export function ensureWebTracer(enabled: boolean): void {
  if (!enabled) {
    return;
  }
  if (!propagatorRegistered) {
    propagation.setGlobalPropagator(new W3CTraceContextPropagator());
    propagatorRegistered = true;
  }
  if (initializedProvider || typeof window === "undefined") {
    return;
  }
  const provider = new WebTracerProvider();
  provider.register();
  initializedProvider = provider;
}

export function resetWebTracerForTests(config?: WebTracerConfig): void {
  void initializedProvider?.shutdown();
  trace.disable();
  initializedProvider = null;
  propagatorRegistered = false;
  if (config) {
    propagation.setGlobalPropagator(new W3CTraceContextPropagator());
    propagatorRegistered = true;
    const provider = new WebTracerProvider(config);
    provider.register();
    initializedProvider = provider;
  }
}

/**
 * W3C `traceparent` / `tracestate` ヘッダを付与してよい送信先か判定する。
 *
 * CORS preflight 失敗や SigV4 署名破損を防ぐため、Backend API (`/api/...`) のみ許可し、
 * direct Electric・S3/R2 Presigned URL・Summaly・Media Proxy・OIDC IdP や
 * `/api/telemetry` 自身には絶対に付与しない。
 */
export function shouldPropagateTraceHeader(
  targetUrl: string,
  apiBaseUrl: string,
  originOverride?: string,
): boolean {
  const origin = currentOrigin(originOverride);
  let target: URL;
  let apiOrigin: string;
  try {
    target = new URL(targetUrl, origin);
    apiOrigin = apiBaseUrl ? new URL(apiBaseUrl, origin).origin : origin;
  } catch {
    return false;
  }
  if (target.origin !== apiOrigin) {
    return false;
  }
  if (!target.pathname.startsWith("/api/")) {
    return false;
  }
  return target.pathname !== "/api/telemetry";
}

export interface ActiveTraceIds {
  traceId?: string;
  spanId?: string;
}

function extractTraceIds(spanContext: SpanContext): ActiveTraceIds {
  if (!trace.isSpanContextValid(spanContext)) {
    return {};
  }
  return {
    traceId: spanContext.traceId,
    spanId: spanContext.spanId,
  };
}

export function getActiveTraceIds(): ActiveTraceIds {
  const activeSpan = trace.getActiveSpan();
  if (!activeSpan) {
    return {};
  }
  return extractTraceIds(activeSpan.spanContext());
}

export interface HttpSpanControl {
  setResponseStatus: (status: number) => void;
}

/**
 * Backend API 呼び出し向けの HTTP クライアントスパンを開始し、対象 URL であれば
 * `traceparent` / `tracestate` を `headers` へ注入する。
 */
export async function runWithHttpSpan<T>(
  options: {
    enabled: boolean;
    method: string;
    url: string;
    apiBaseUrl: string;
    headers: Headers;
  },
  fn: (traceIds: ActiveTraceIds, control: HttpSpanControl) => Promise<T>,
): Promise<T> {
  const noopControl: HttpSpanControl = {
    setResponseStatus: () => {},
  };
  if (!options.enabled || !shouldPropagateTraceHeader(options.url, options.apiBaseUrl)) {
    return fn({}, noopControl);
  }

  ensureWebTracer(true);
  const tracer = trace.getTracer(TRACER_NAME);

  return tracer.startActiveSpan(
    `HTTP ${options.method.toUpperCase()}`,
    {
      kind: SpanKind.CLIENT,
      attributes: {
        "http.request.method": options.method.toUpperCase(),
      },
    },
    async (span) => {
      let responseStatus: number | undefined;
      const control: HttpSpanControl = {
        setResponseStatus(status: number) {
          responseStatus = status;
        },
      };
      try {
        propagation.inject(context.active(), options.headers, headerSetter);
        const traceIds = extractTraceIds(span.spanContext());
        const result = await fn(traceIds, control);
        const status = responseStatus ?? (result instanceof Response ? result.status : undefined);
        if (status !== undefined) {
          span.setAttribute("http.response.status_code", status);
        }
        if (status !== undefined && status >= 500) {
          span.setStatus({
            code: SpanStatusCode.ERROR,
            message: `HTTP ${status}`,
          });
        } else {
          span.setStatus({ code: SpanStatusCode.OK });
        }
        return result;
      } catch (error) {
        if (responseStatus !== undefined) {
          span.setAttribute("http.response.status_code", responseStatus);
        }
        span.setStatus({
          code: SpanStatusCode.ERROR,
          message: error instanceof Error ? error.message : String(error),
        });
        throw error;
      } finally {
        span.end();
      }
    },
  );
}
