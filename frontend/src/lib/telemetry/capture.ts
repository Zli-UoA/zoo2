import type { AppConfig } from "../config";
import {
  getTelemetryReporter,
  type TelemetryCandidateType,
  type TelemetryElectricShape,
  type TelemetryElectricSyncState,
  type TelemetryWebVitalRating,
  type TelemetryWebrtcState,
} from "./reporter";
import { ensureWebTracer, getActiveTraceIds, type ActiveTraceIds } from "./tracer";

const MAX_MESSAGE_LENGTH = 200;

let uninstallActive: (() => void) | null = null;

export function normalizeRouteGroup(pathname?: string): string {
  const path = pathname ?? (typeof window !== "undefined" ? window.location.pathname : "/");
  const clean = path.split(/[?#]/)[0]?.trim() || "/";
  if (clean === "/") return "/";
  for (const prefix of [
    "/channels",
    "/home",
    "/dms",
    "/notifications",
    "/settings",
    "/public",
    "/auth",
    "/apps",
  ]) {
    if (clean.startsWith(prefix)) {
      return prefix;
    }
  }
  return "other";
}

export function sanitizeErrorMessage(error: unknown): string {
  const raw =
    error instanceof Error
      ? `${error.name}: ${error.message}`
      : typeof error === "string"
        ? error
        : "Unknown error";

  const masked = raw
    .replace(/[\r\n\t]+/g, " ")
    .replace(/Bearer\s+\S+/gi, "Bearer [redacted]")
    .replace(/(https?:\/\/[^\s?]+)\?[^\s]*/gi, "$1?[redacted]")
    .replace(/(\/api\/[^\s?]+)\?[^\s]*/gi, "$1?[redacted]")
    .trim();

  return masked.slice(0, MAX_MESSAGE_LENGTH);
}

export function inferElectricShape(urlOrPath: string): TelemetryElectricShape {
  const lowered = urlOrPath.toLowerCase();
  if (lowered.includes("message_reactions")) return "message_reactions";
  if (lowered.includes("channel_users")) return "channel_users";
  if (lowered.includes("notifications")) return "notifications";
  if (lowered.includes("channels")) return "channels";
  if (lowered.includes("messages")) return "messages";
  if (lowered.includes("users")) return "users";
  return "other";
}

function vitalRating(
  metric: "lcp" | "inp" | "ttfb" | "fcp",
  valueMs: number,
): TelemetryWebVitalRating {
  const thresholds: Record<typeof metric, [number, number]> = {
    lcp: [2500, 4000],
    inp: [200, 500],
    ttfb: [800, 1800],
    fcp: [1800, 3000],
  };
  const [good, poor] = thresholds[metric];
  if (valueMs <= good) return "good";
  if (valueMs <= poor) return "needs_improvement";
  return "poor";
}

export function reportApiError(params: {
  httpStatus?: number;
  durationMs?: number;
  error?: unknown;
  traceIds?: ActiveTraceIds;
}): void {
  const traceIds = params.traceIds ?? getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "error",
    category: "api",
    route: normalizeRouteGroup(),
    http_status: params.httpStatus,
    duration_ms: params.durationMs,
    message: params.error ? sanitizeErrorMessage(params.error) : undefined,
    trace_id: traceIds.traceId,
    span_id: traceIds.spanId,
  });
}

export function reportReactBoundaryError(error: unknown): void {
  const traceIds = getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "error",
    category: "react_boundary",
    route: normalizeRouteGroup(),
    message: sanitizeErrorMessage(error),
    trace_id: traceIds.traceId,
    span_id: traceIds.spanId,
  });
}

export function reportElectricSyncEvent(params: {
  shape: TelemetryElectricShape;
  state: TelemetryElectricSyncState;
  httpStatus?: number;
  traceIds?: ActiveTraceIds;
}): void {
  const traceIds = params.traceIds ?? getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "electric_sync",
    shape: params.shape,
    state: params.state,
    http_status: params.httpStatus,
    route: normalizeRouteGroup(),
    trace_id: traceIds.traceId,
    span_id: traceIds.spanId,
  });
}

export function reportWebrtcEvent(params: {
  state: TelemetryWebrtcState;
  candidateType?: TelemetryCandidateType;
  rttMs?: number;
  lossRatio?: number;
}): void {
  const traceIds = getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "webrtc",
    state: params.state,
    candidate_type: params.candidateType,
    rtt_ms: params.rttMs,
    loss_ratio: params.lossRatio,
    trace_id: traceIds.traceId,
    span_id: traceIds.spanId,
  });
}

export function installBrowserTelemetry(
  config: Pick<AppConfig, "apiBaseUrl" | "telemetryEnabled">,
): () => void {
  if (uninstallActive) {
    return uninstallActive;
  }

  const reporter = getTelemetryReporter();
  reporter.configure({
    apiBaseUrl: config.apiBaseUrl,
    enabled: config.telemetryEnabled,
  });

  if (!config.telemetryEnabled || typeof window === "undefined") {
    return () => {};
  }

  ensureWebTracer(true);

  let latestLcpMs: number | null = null;
  let latestLcpRoute: string | null = null;
  let lcpReported = false;
  const interactionsById = new Map<number, number>();
  let fallbackMaxInpMs: number | null = null;
  let latestInpRoute: string | null = null;
  let lastFlushedInpMs: number | null = null;
  let fcpReported = false;
  let ttfbReported = false;

  const computeInpMs = (): number | null => {
    if (interactionsById.size > 0) {
      const sorted = [...interactionsById.values()].sort((a, b) => b - a);
      const p98Index = Math.min(sorted.length - 1, Math.floor(sorted.length / 50));
      return sorted[p98Index] ?? null;
    }
    return fallbackMaxInpMs;
  };

  const flushVitals = () => {
    const currentRoute = normalizeRouteGroup();
    if (latestLcpMs !== null && !lcpReported) {
      lcpReported = true;
      reporter.enqueue({
        kind: "web_vital",
        metric: "lcp",
        value_ms: Math.round(latestLcpMs),
        rating: vitalRating("lcp", latestLcpMs),
        route: latestLcpRoute ?? currentRoute,
      });
    }
    const currentInpMs = computeInpMs();
    if (currentInpMs !== null && currentInpMs !== lastFlushedInpMs) {
      lastFlushedInpMs = currentInpMs;
      reporter.enqueue({
        kind: "web_vital",
        metric: "inp",
        value_ms: Math.round(currentInpMs),
        rating: vitalRating("inp", currentInpMs),
        route: latestInpRoute ?? currentRoute,
      });
    }
  };

  const onError = (event: ErrorEvent) => {
    reporter.enqueue({
      kind: "error",
      category: "runtime",
      route: normalizeRouteGroup(),
      message: sanitizeErrorMessage(event.error ?? event.message),
    });
  };

  const onRejection = (event: PromiseRejectionEvent) => {
    reporter.enqueue({
      kind: "error",
      category: "unhandled_rejection",
      route: normalizeRouteGroup(),
      message: sanitizeErrorMessage(event.reason),
    });
  };

  const onVisibilityOrNetwork = () => {
    if (typeof document !== "undefined" && document.visibilityState === "hidden") {
      flushVitals();
    }
    reporter.handleVisibilityOrNetworkChange();
  };

  const onPageHide = () => {
    flushVitals();
    void reporter.flush({ keepalive: true });
  };

  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
  window.addEventListener("pagehide", onPageHide);
  window.addEventListener("online", onVisibilityOrNetwork);
  window.addEventListener("offline", onVisibilityOrNetwork);
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", onVisibilityOrNetwork);
  }

  const observers: PerformanceObserver[] = [];
  if (typeof PerformanceObserver !== "undefined") {
    try {
      const lcpObserver = new PerformanceObserver((list) => {
        const entries = list.getEntries();
        const last = entries.at(-1);
        if (last && Number.isFinite(last.startTime) && last.startTime >= 0) {
          latestLcpMs = last.startTime;
          latestLcpRoute = normalizeRouteGroup();
        }
      });
      lcpObserver.observe({ type: "largest-contentful-paint", buffered: true });
      observers.push(lcpObserver);
    } catch {
      // Unsupported entry type
    }

    try {
      const paintObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (entry.name === "first-contentful-paint" && !fcpReported && entry.startTime >= 0) {
            fcpReported = true;
            reporter.enqueue({
              kind: "web_vital",
              metric: "fcp",
              value_ms: Math.round(entry.startTime),
              rating: vitalRating("fcp", entry.startTime),
              route: normalizeRouteGroup(),
            });
          }
        }
      });
      paintObserver.observe({ type: "paint", buffered: true });
      observers.push(paintObserver);
    } catch {
      // Unsupported entry type
    }

    try {
      const navObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (ttfbReported) continue;
          const nav = entry as PerformanceNavigationTiming & { activationStart?: number };
          const activationStart =
            typeof nav.activationStart === "number" && Number.isFinite(nav.activationStart)
              ? Math.max(0, nav.activationStart)
              : 0;
          if (
            typeof nav.responseStart === "number" &&
            Number.isFinite(nav.responseStart) &&
            nav.responseStart >= 0
          ) {
            const ttfb = Math.max(0, nav.responseStart - activationStart);
            ttfbReported = true;
            reporter.enqueue({
              kind: "web_vital",
              metric: "ttfb",
              value_ms: Math.round(ttfb),
              rating: vitalRating("ttfb", ttfb),
              route: normalizeRouteGroup(),
            });
          }
        }
      });
      navObserver.observe({ type: "navigation", buffered: true });
      observers.push(navObserver);
    } catch {
      // Unsupported entry type
    }

    try {
      const eventObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const eventEntry = entry as PerformanceEntry & { interactionId?: number };
          const duration = entry.duration;
          if (!Number.isFinite(duration) || duration < 0) {
            continue;
          }
          if (typeof eventEntry.interactionId === "number" && eventEntry.interactionId > 0) {
            const prev = interactionsById.get(eventEntry.interactionId) ?? 0;
            if (duration > prev) {
              interactionsById.set(eventEntry.interactionId, duration);
              latestInpRoute = normalizeRouteGroup();
            }
          } else if (eventEntry.interactionId === undefined) {
            fallbackMaxInpMs =
              fallbackMaxInpMs === null ? duration : Math.max(fallbackMaxInpMs, duration);
            latestInpRoute = normalizeRouteGroup();
          }
        }
      });
      eventObserver.observe({
        type: "event",
        buffered: true,
        durationThreshold: 16,
      } as PerformanceObserverInit & { durationThreshold?: number });
      observers.push(eventObserver);
    } catch {
      // Unsupported entry type
    }
  }

  const uninstall = () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
    window.removeEventListener("pagehide", onPageHide);
    window.removeEventListener("online", onVisibilityOrNetwork);
    window.removeEventListener("offline", onVisibilityOrNetwork);
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", onVisibilityOrNetwork);
    }
    for (const observer of observers) {
      observer.disconnect();
    }
    uninstallActive = null;
  };

  uninstallActive = uninstall;
  return uninstall;
}
