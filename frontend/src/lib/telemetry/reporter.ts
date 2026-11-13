import { joinUrl } from "../config";

export type TelemetryErrorCategory =
  | "runtime"
  | "unhandled_rejection"
  | "react_boundary"
  | "api"
  | "electric"
  | "webrtc";

export type TelemetryWebVitalMetric = "lcp" | "inp" | "ttfb" | "fcp";
export type TelemetryWebVitalRating = "good" | "needs_improvement" | "poor";

export type TelemetryElectricShape =
  | "channels"
  | "users"
  | "channel_users"
  | "messages"
  | "message_reactions"
  | "notifications"
  | "other";

export type TelemetryElectricSyncState = "error" | "stalled" | "reconnected" | "offline_fallback";

export type TelemetryWebrtcState =
  | "ice_failed"
  | "connection_failed"
  | "heartbeat_timeout"
  | "poor_quality"
  | "recovered";

export type TelemetryCandidateType = "host" | "srflx" | "prflx" | "relay" | "unknown";

export type FrontendTelemetryEvent =
  | {
      kind: "error";
      category: TelemetryErrorCategory;
      route?: string;
      http_status?: number;
      duration_ms?: number;
      message?: string;
      trace_id?: string;
      span_id?: string;
    }
  | {
      kind: "web_vital";
      metric: TelemetryWebVitalMetric;
      value_ms: number;
      rating?: TelemetryWebVitalRating;
      route?: string;
      trace_id?: string;
      span_id?: string;
    }
  | {
      kind: "electric_sync";
      shape: TelemetryElectricShape;
      state: TelemetryElectricSyncState;
      http_status?: number;
      route?: string;
      trace_id?: string;
      span_id?: string;
    }
  | {
      kind: "webrtc";
      state: TelemetryWebrtcState;
      candidate_type?: TelemetryCandidateType;
      rtt_ms?: number;
      loss_ratio?: number;
      trace_id?: string;
      span_id?: string;
    };

export interface TelemetryReporterOptions {
  apiBaseUrl?: string;
  enabled?: boolean;
  flushIntervalMs?: number;
  maxBatchEvents?: number;
  maxQueueSize?: number;
  dedupWindowMs?: number;
  fetchImpl?: typeof fetch;
  now?: () => number;
  isHidden?: () => boolean;
  isOnline?: () => boolean;
}

const DEFAULT_FLUSH_INTERVAL_MS = 15_000;
const DEFAULT_MAX_BATCH_EVENTS = 20;
const DEFAULT_MAX_QUEUE_SIZE = 100;
const DEFAULT_DEDUP_WINDOW_MS = 10_000;
const MAX_BACKOFF_MS = 300_000;

function defaultIsHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}

function defaultIsOnline(): boolean {
  return typeof navigator === "undefined" || navigator.onLine !== false;
}

function dedupKey(event: FrontendTelemetryEvent): string {
  switch (event.kind) {
    case "error":
      return `error:${event.category}:${event.route ?? "other"}:${event.http_status ?? 0}`;
    case "web_vital":
      return `vital:${event.metric}:${event.route ?? "other"}`;
    case "electric_sync":
      return `electric:${event.shape}:${event.state}:${event.http_status ?? 0}`;
    case "webrtc":
      return `webrtc:${event.state}`;
  }
}

/**
 * `docs/request-design.md` に従った共有テレメトリバッチャ。
 *
 * キューが空・タブ非表示・オフライン時はタイマーを停止し、
 * `visibilitychange(hidden)` / `pagehide` 時は `keepalive: true` でフラッシュする。
 * 送信自身の失敗時は再エラー化・無限ループを起こさず静かにバックオフする。
 */
export class TelemetryReporter {
  private apiBaseUrl: string;
  private enabled: boolean;
  private readonly flushIntervalMs: number;
  private readonly maxBatchEvents: number;
  private readonly maxQueueSize: number;
  private readonly dedupWindowMs: number;
  private readonly fetchImpl: typeof fetch;
  private readonly now: () => number;
  private readonly isHidden: () => boolean;
  private readonly isOnline: () => boolean;

  private queue: FrontendTelemetryEvent[] = [];
  private recentKeys = new Map<string, number>();
  private timerId: ReturnType<typeof setTimeout> | null = null;
  private inFlight = false;
  private failureCount = 0;
  private backoffUntil = 0;

  constructor(options: TelemetryReporterOptions = {}) {
    this.apiBaseUrl = options.apiBaseUrl ?? "";
    this.enabled = options.enabled ?? true;
    this.flushIntervalMs = options.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS;
    this.maxBatchEvents = options.maxBatchEvents ?? DEFAULT_MAX_BATCH_EVENTS;
    this.maxQueueSize = options.maxQueueSize ?? DEFAULT_MAX_QUEUE_SIZE;
    this.dedupWindowMs = options.dedupWindowMs ?? DEFAULT_DEDUP_WINDOW_MS;
    this.fetchImpl = options.fetchImpl ?? ((input, init) => globalThis.fetch(input, init));
    this.now = options.now ?? (() => Date.now());
    this.isHidden = options.isHidden ?? defaultIsHidden;
    this.isOnline = options.isOnline ?? defaultIsOnline;
  }

  configure(options: Pick<TelemetryReporterOptions, "apiBaseUrl" | "enabled">): void {
    if (options.apiBaseUrl !== undefined) {
      this.apiBaseUrl = options.apiBaseUrl;
    }
    if (options.enabled !== undefined) {
      this.enabled = options.enabled;
      if (!this.enabled) {
        this.queue = [];
        this.stopTimer();
      }
    }
  }

  pendingCount(): number {
    return this.queue.length;
  }

  enqueue(event: FrontendTelemetryEvent): void {
    if (!this.enabled) {
      return;
    }
    const now = this.now();
    const key = dedupKey(event);
    const prev = this.recentKeys.get(key);
    if (prev !== undefined && now - prev < this.dedupWindowMs) {
      return;
    }
    this.recentKeys.set(key, now);
    if (this.recentKeys.size > 128) {
      for (const [k, ts] of this.recentKeys) {
        if (now - ts >= this.dedupWindowMs) {
          this.recentKeys.delete(k);
        }
      }
    }

    if (this.queue.length >= this.maxQueueSize) {
      this.queue.shift();
    }
    this.queue.push(event);

    if (this.queue.length >= this.maxBatchEvents && !this.isHidden() && this.isOnline()) {
      void this.flush();
      return;
    }
    this.scheduleIfActive();
  }

  handleVisibilityOrNetworkChange(): void {
    if (this.isHidden() || !this.isOnline()) {
      this.stopTimer();
      if (this.isHidden() && this.isOnline() && this.queue.length > 0) {
        void this.flush({ keepalive: true });
      }
      return;
    }
    this.scheduleIfActive();
  }

  async flush(options: { keepalive?: boolean } = {}): Promise<void> {
    const keepalive = options.keepalive ?? false;
    if (!this.enabled || this.queue.length === 0) {
      this.stopTimer();
      return;
    }
    if (!keepalive) {
      if (this.inFlight || !this.isOnline() || this.now() < this.backoffUntil) {
        return;
      }
    }

    this.stopTimer();
    const batch = this.queue.splice(0, this.maxBatchEvents);
    const endpoint = joinUrl(this.apiBaseUrl, "/api/telemetry");
    this.inFlight = true;

    try {
      const response = await this.fetchImpl(endpoint, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ events: batch }),
        keepalive,
      });
      if (!response.ok) {
        this.recordFailure();
      } else {
        this.failureCount = 0;
        this.backoffUntil = 0;
      }
    } catch {
      this.recordFailure();
    } finally {
      this.inFlight = false;
      this.scheduleIfActive();
    }
  }

  dispose(): void {
    this.stopTimer();
    this.queue = [];
    this.recentKeys.clear();
  }

  private recordFailure(): void {
    this.failureCount = Math.min(this.failureCount + 1, 5);
    const delay = Math.min(this.flushIntervalMs * 2 ** (this.failureCount - 1), MAX_BACKOFF_MS);
    this.backoffUntil = this.now() + delay;
  }

  private scheduleIfActive(): void {
    if (
      !this.enabled ||
      this.queue.length === 0 ||
      this.timerId !== null ||
      this.isHidden() ||
      !this.isOnline()
    ) {
      return;
    }
    const waitMs = Math.max(this.flushIntervalMs, this.backoffUntil - this.now());
    this.timerId = setTimeout(() => {
      this.timerId = null;
      void this.flush();
    }, waitMs);
  }

  private stopTimer(): void {
    if (this.timerId !== null) {
      clearTimeout(this.timerId);
      this.timerId = null;
    }
  }
}

let sharedReporter: TelemetryReporter | null = null;

export function getTelemetryReporter(): TelemetryReporter {
  if (!sharedReporter) {
    sharedReporter = new TelemetryReporter();
  }
  return sharedReporter;
}

export function resetTelemetryReporterForTests(
  options?: TelemetryReporterOptions,
): TelemetryReporter {
  sharedReporter?.dispose();
  sharedReporter = new TelemetryReporter(options);
  return sharedReporter;
}
