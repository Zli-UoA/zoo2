import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { resolvePublicConfig, resolveTelemetryEnabled } from "../config";
import { createApiClient } from "../api/client";
import { createShapeFetch } from "../electric/headers";
import { inferElectricShape, normalizeRouteGroup, sanitizeErrorMessage } from "./capture";
import { TelemetryReporter, resetTelemetryReporterForTests } from "./reporter";
import { shouldPropagateTraceHeader } from "./tracer";

describe("shouldPropagateTraceHeader", () => {
  it("allows only Backend /api/ requests on the configured API origin", () => {
    expect(
      shouldPropagateTraceHeader(
        "http://localhost:8000/api/messages",
        "http://localhost:8000",
        "http://localhost:5173",
      ),
    ).toBe(true);
    expect(shouldPropagateTraceHeader("/api/channels", "", "https://zoo.example.com")).toBe(true);

    // /api/telemetry 自身・direct Electric・R2・OIDC・Summaly は対象外
    expect(
      shouldPropagateTraceHeader(
        "http://localhost:8000/api/telemetry",
        "http://localhost:8000",
        "http://localhost:5173",
      ),
    ).toBe(false);
    expect(
      shouldPropagateTraceHeader(
        "http://localhost:3000/v1/shape?table=messages",
        "http://localhost:8000",
        "http://localhost:5173",
      ),
    ).toBe(false);
    expect(
      shouldPropagateTraceHeader(
        "https://bucket.r2.cloudflarestorage.com/obj?X-Amz-Signature=abc",
        "",
        "https://zoo.example.com",
      ),
    ).toBe(false);
    expect(
      shouldPropagateTraceHeader(
        "https://idm.example.com/oauth2/token",
        "",
        "https://zoo.example.com",
      ),
    ).toBe(false);
  });
});

describe("capture helpers", () => {
  it("normalizes routes into bounded groups without IDs or queries", () => {
    expect(normalizeRouteGroup("/")).toBe("/");
    expect(normalizeRouteGroup("/channels/0193-abc?thread=1")).toBe("/channels");
    expect(normalizeRouteGroup("/public/users/alice")).toBe("/public");
    expect(normalizeRouteGroup("/unknown/route")).toBe("other");
  });

  it("masks Bearer tokens and URL query strings in error messages", () => {
    const sanitized = sanitizeErrorMessage(
      new Error(
        "Failed\nBearer secret-token-123 at https://example.com/api/shapes/messages?secret=xyz",
      ),
    );
    expect(sanitized).not.toContain("secret-token-123");
    expect(sanitized).not.toContain("secret=xyz");
    expect(sanitized).toContain("Bearer [redacted]");
    expect(sanitized).toContain("https://example.com/api/shapes/messages?[redacted]");
  });

  it("infers Electric shapes accurately", () => {
    expect(inferElectricShape("/api/shapes/messages?offset=-1")).toBe("messages");
    expect(inferElectricShape("/api/shapes/message_reactions")).toBe("message_reactions");
    expect(inferElectricShape("/api/shapes/channel_users")).toBe("channel_users");
    expect(inferElectricShape("http://localhost:3000/v1/shape?table=channels")).toBe("channels");
  });
});

describe("TelemetryReporter", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("does nothing when queue is empty and batches events on interval", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 202 }));
    const reporter = new TelemetryReporter({
      apiBaseUrl: "http://localhost:8000",
      fetchImpl: fetchMock,
      flushIntervalMs: 1000,
      isHidden: () => false,
      isOnline: () => true,
    });

    await reporter.flush();
    expect(fetchMock).not.toHaveBeenCalled();

    reporter.enqueue({ kind: "error", category: "runtime", route: "/channels" });
    reporter.enqueue({ kind: "web_vital", metric: "lcp", value_ms: 1100, route: "/channels" });
    expect(reporter.pendingCount()).toBe(2);

    await vi.advanceTimersByTimeAsync(1000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://localhost:8000/api/telemetry");
    expect(init?.method).toBe("POST");
    const bodyStr = typeof init?.body === "string" ? init.body : "";
    const parsed = JSON.parse(bodyStr) as { events: unknown[] };
    expect(parsed.events).toHaveLength(2);
    expect(reporter.pendingCount()).toBe(0);
    reporter.dispose();
  });

  it("deduplicates identical events within the dedup window and flushes with keepalive when hidden", async () => {
    let hidden = false;
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 202 }));
    const reporter = new TelemetryReporter({
      apiBaseUrl: "",
      fetchImpl: fetchMock,
      dedupWindowMs: 10_000,
      isHidden: () => hidden,
      isOnline: () => true,
    });

    reporter.enqueue({ kind: "error", category: "api", route: "/home", http_status: 500 });
    reporter.enqueue({ kind: "error", category: "api", route: "/home", http_status: 500 });
    expect(reporter.pendingCount()).toBe(1);

    hidden = true;
    reporter.handleVisibilityOrNetworkChange();
    await Promise.resolve();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[1]?.keepalive).toBe(true);
    reporter.dispose();
  });

  it("backs off silently without throwing when telemetry endpoint fails", async () => {
    let now = 1000;
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 503 }));
    const reporter = new TelemetryReporter({
      fetchImpl: fetchMock,
      flushIntervalMs: 1000,
      now: () => now,
      isHidden: () => false,
      isOnline: () => true,
    });

    reporter.enqueue({ kind: "webrtc", state: "ice_failed" });
    await reporter.flush();
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // バックオフ中は再送をスキップする
    now += 500;
    reporter.enqueue({ kind: "webrtc", state: "connection_failed" });
    await reporter.flush();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    reporter.dispose();
  });
});

describe("API and Electric client trace propagation", () => {
  it("injects traceparent on Backend API calls in browser environment and never on direct Electric", async () => {
    vi.stubGlobal("window", {
      location: { origin: "http://localhost:5173", pathname: "/channels/1" },
    });
    try {
      const apiCalls: RequestInit[] = [];
      const client = createApiClient({
        baseUrl: "http://localhost:8000",
        fetch: async (_url, init) => {
          apiCalls.push(init ?? {});
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          });
        },
      });
      await client.get("/api/channels");
      const apiTraceparent = new Headers(apiCalls[0]?.headers).get("traceparent");
      expect(apiTraceparent).toMatch(/^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/);

      const directCalls: RequestInit[] = [];
      const directFetch = createShapeFetch({
        auth: false,
        apiBaseUrl: "http://localhost:8000",
        fetch: async (_input, init) => {
          directCalls.push(init ?? {});
          return new Response(null, { status: 200 });
        },
      });
      await directFetch("http://localhost:3000/v1/shape?table=messages");
      expect(new Headers(directCalls[0]?.headers).get("traceparent")).toBeNull();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("reports 5xx API errors via shared telemetry reporter", async () => {
    const telemetryCalls: string[] = [];
    const reporter = resetTelemetryReporterForTests({
      apiBaseUrl: "http://localhost:8000",
      fetchImpl: async (_input, init) => {
        telemetryCalls.push(typeof init?.body === "string" ? init.body : "");
        return new Response(null, { status: 202 });
      },
      isHidden: () => false,
      isOnline: () => true,
    });

    const client = createApiClient({
      baseUrl: "http://localhost:8000",
      fetch: async () => new Response(JSON.stringify({ error: "bad" }), { status: 502 }),
    });

    await expect(client.get("/api/channels")).rejects.toThrow();
    expect(reporter.pendingCount()).toBe(1);
    await reporter.flush();
    expect(telemetryCalls).toHaveLength(1);
    expect(telemetryCalls[0]).toContain('"category":"api"');
    expect(telemetryCalls[0]).toContain('"http_status":502');
    reporter.dispose();
  });

  it("respects VITE_TELEMETRY_ENABLED=false", () => {
    expect(resolveTelemetryEnabled(undefined)).toBe(true);
    expect(resolveTelemetryEnabled("false")).toBe(false);
    expect(resolvePublicConfig({ VITE_TELEMETRY_ENABLED: "false" }).telemetryEnabled).toBe(false);
  });
});
