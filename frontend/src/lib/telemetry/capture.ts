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
  if (lowered.includes("attachments")) return "attachments";
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
  route?: string;
  traceIds?: ActiveTraceIds;
}): void {
  const traceIds = params.traceIds ?? getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "error",
    category: "api",
    route: params.route ? normalizeRouteGroup(params.route) : normalizeRouteGroup(),
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
  route?: string;
  traceIds?: ActiveTraceIds;
}): void {
  const traceIds = params.traceIds ?? getActiveTraceIds();
  getTelemetryReporter().enqueue({
    kind: "electric_sync",
    shape: params.shape,
    state: params.state,
    http_status: params.httpStatus,
    route: params.route ? normalizeRouteGroup(params.route) : normalizeRouteGroup(),
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
  const interactionsById = new Map<number, { duration: number; route: string }>();
  let minInteractionId = Infinity;
  let maxInteractionId = 0;
  let fallbackMaxInp: { duration: number; route: string } | null = null;
  let lastFlushedInpMs: number | null = null;
  let lastFlushedInpRoute: string | null = null;
  let fcpReported = false;
  let ttfbReported = false;

  const computeInp = (): { duration: number; route: string } | null => {
    if (interactionsById.size > 0) {
      const sorted = [...interactionsById.values()].sort((a, b) => b.duration - a.duration);
      const perfInteractionCount =
        typeof performance !== "undefined" &&
        typeof (performance as Performance & { interactionCount?: number }).interactionCount ===
          "number" &&
        Number.isFinite(
          (performance as Performance & { interactionCount?: number }).interactionCount,
        )
          ? Math.max(
              0,
              (performance as Performance & { interactionCount?: number }).interactionCount ?? 0,
            )
          : undefined;
      const estimatedFromIds =
        perfInteractionCount === undefined &&
        Number.isFinite(minInteractionId) &&
        maxInteractionId > minInteractionId
          ? Math.floor((maxInteractionId - minInteractionId) / 7) + 1
          : 0;
      const totalInteractions = Math.max(
        perfInteractionCount ?? 0,
        estimatedFromIds,
        sorted.length,
      );
      const p98Index = Math.min(sorted.length - 1, Math.floor(totalInteractions / 50));
      return sorted[p98Index] ?? null;
    }
    return fallbackMaxInp;
  };

  const observerEntries: Array<{
    observer: PerformanceObserver;
    process: (entries: PerformanceEntry[]) => void;
  }> = [];

  const flushVitals = () => {
    for (const { observer, process } of observerEntries) {
      if (typeof observer.takeRecords === "function") {
        const pending = observer.takeRecords();
        if (pending.length > 0) {
          process(pending);
        }
      }
    }
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
    const currentInp = computeInp();
    if (
      currentInp !== null &&
      (currentInp.duration !== lastFlushedInpMs || currentInp.route !== lastFlushedInpRoute)
    ) {
      const accepted = reporter.enqueue({
        kind: "web_vital",
        metric: "inp",
        value_ms: Math.round(currentInp.duration),
        rating: vitalRating("inp", currentInp.duration),
        route: currentInp.route,
      });
      if (accepted) {
        lastFlushedInpMs = currentInp.duration;
        lastFlushedInpRoute = currentInp.route;
      }
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

  if (typeof PerformanceObserver !== "undefined") {
    const observeWithHandler = (
      options: PerformanceObserverInit & { durationThreshold?: number },
      process: (entries: PerformanceEntry[]) => void,
    ) => {
      try {
        const observer = new PerformanceObserver((list) => {
          process(list.getEntries());
        });
        observer.observe(options);
        observerEntries.push({ observer, process });
      } catch {
        // Unsupported entry type
      }
    };

    observeWithHandler({ type: "largest-contentful-paint", buffered: true }, (entries) => {
      const last = entries.at(-1);
      if (last && Number.isFinite(last.startTime) && last.startTime >= 0) {
        latestLcpMs = last.startTime;
        latestLcpRoute = normalizeRouteGroup();
      }
    });

    observeWithHandler({ type: "paint", buffered: true }, (entries) => {
      for (const entry of entries) {
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

    observeWithHandler({ type: "navigation", buffered: true }, (entries) => {
      for (const entry of entries) {
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

    observeWithHandler(
      {
        type: "event",
        buffered: true,
        durationThreshold: 16,
      },
      (entries) => {
        const routeAtObservation = normalizeRouteGroup();
        for (const entry of entries) {
          const eventEntry = entry as PerformanceEntry & { interactionId?: number };
          const duration = entry.duration;
          if (!Number.isFinite(duration) || duration < 0) {
            continue;
          }
          if (typeof eventEntry.interactionId === "number" && eventEntry.interactionId > 0) {
            minInteractionId = Math.min(minInteractionId, eventEntry.interactionId);
            maxInteractionId = Math.max(maxInteractionId, eventEntry.interactionId);
            const prev = interactionsById.get(eventEntry.interactionId);
            if (!prev || duration > prev.duration) {
              interactionsById.set(eventEntry.interactionId, {
                duration,
                route: routeAtObservation,
              });
            }
          } else if (eventEntry.interactionId === undefined) {
            if (!fallbackMaxInp || duration > fallbackMaxInp.duration) {
              fallbackMaxInp = { duration, route: routeAtObservation };
            }
          }
        }
      },
    );
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
    for (const { observer } of observerEntries) {
      observer.disconnect();
    }
    uninstallActive = null;
  };

  uninstallActive = uninstall;
  return uninstall;
}
