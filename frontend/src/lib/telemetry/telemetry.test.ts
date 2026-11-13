import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { resolvePublicConfig, resolveTelemetryEnabled } from "../config";
import { createApiClient } from "../api/client";
import { createShapeFetch } from "../electric/headers";
import {
  inferElectricShape,
  installBrowserTelemetry,
  normalizeRouteGroup,
  sanitizeErrorMessage,
} from "./capture";
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

  it("requeues failed batch for retry after backoff expires and separates distinct error messages", async () => {
    let now = 1000;
    let shouldFail = true;
    const deliveredBatches: Array<Array<{ kind: string; message?: string }>> = [];
    const fetchMock = vi.fn<typeof fetch>(async (_input, init) => {
      if (shouldFail) {
        return new Response(null, { status: 503 });
      }
      const parsed = JSON.parse(typeof init?.body === "string" ? init.body : "{}") as {
        events: Array<{ kind: string; message?: string }>;
      };
      deliveredBatches.push(parsed.events);
      return new Response(null, { status: 202 });
    });

    const reporter = new TelemetryReporter({
      fetchImpl: fetchMock,
      flushIntervalMs: 1000,
      dedupWindowMs: 10_000,
      now: () => now,
      isHidden: () => false,
      isOnline: () => true,
    });

    reporter.enqueue({
      kind: "error",
      category: "api",
      route: "/home",
      http_status: 500,
      message: "Request timed out",
    });
    reporter.enqueue({
      kind: "error",
      category: "api",
      route: "/home",
      http_status: 500,
      message: "Internal error",
    });
    expect(reporter.pendingCount()).toBe(2);

    await reporter.flush();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(reporter.pendingCount()).toBe(2);

    shouldFail = false;
    now += 20_000;
    await reporter.flush();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(reporter.pendingCount()).toBe(0);
    expect(deliveredBatches[0]).toHaveLength(2);
    reporter.dispose();
  });

  it("drops permanently rejected 4xx batches instead of requeuing them forever", async () => {
    const fetchMock = vi.fn<typeof fetch>(async () => new Response(null, { status: 400 }));
    const reporter = new TelemetryReporter({
      fetchImpl: fetchMock,
      isHidden: () => false,
      isOnline: () => true,
    });

    reporter.enqueue({ kind: "error", category: "runtime", route: "/home", message: "bad" });
    await reporter.flush();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(reporter.pendingCount()).toBe(0);
    reporter.dispose();
  });

  it("drains all batches across keepalive flush when queue exceeds single batch size", async () => {
    const sentEvents: unknown[] = [];
    const fetchMock = vi.fn<typeof fetch>(async (_input, init) => {
      const parsed = JSON.parse(typeof init?.body === "string" ? init.body : "{}") as {
        events: unknown[];
      };
      sentEvents.push(...parsed.events);
      return new Response(null, { status: 202 });
    });

    const reporter = new TelemetryReporter({
      fetchImpl: fetchMock,
      maxBatchEvents: 20,
      isHidden: () => true,
      isOnline: () => true,
    });

    for (let i = 0; i < 21; i += 1) {
      reporter.enqueue({
        kind: "error",
        category: "runtime",
        route: "/home",
        message: `err-${i}`,
      });
    }
    expect(reporter.pendingCount()).toBe(21);

    await reporter.flush({ keepalive: true });
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(sentEvents).toHaveLength(21);
    expect(reporter.pendingCount()).toBe(0);
    reporter.dispose();
  });
});

describe("installBrowserTelemetry web vitals", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves LCP route across SPA navigation, includes activationStart in TTFB, and configures INP durationThreshold", async () => {
    const windowListeners = new Map<string, EventListener>();
    const documentListeners = new Map<string, EventListener>();
    const locationObj = { origin: "http://localhost:5173", pathname: "/channels/123" };
    const observerRecords: Array<{
      options: PerformanceObserverInit & { durationThreshold?: number };
      callback: PerformanceObserverCallback;
    }> = [];

    class MockPerformanceObserver {
      private callback: PerformanceObserverCallback;
      constructor(callback: PerformanceObserverCallback) {
        this.callback = callback;
      }
      observe(options: PerformanceObserverInit & { durationThreshold?: number }) {
        observerRecords.push({ options, callback: this.callback });
      }
      disconnect() {}
    }

    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.stubGlobal("window", {
      location: locationObj,
      addEventListener: (type: string, cb: EventListener) => {
        windowListeners.set(type, cb);
      },
      removeEventListener: (type: string) => {
        windowListeners.delete(type);
      },
    });
    vi.stubGlobal("document", {
      visibilityState: "visible",
      addEventListener: (type: string, cb: EventListener) => {
        documentListeners.set(type, cb);
      },
      removeEventListener: (type: string) => {
        documentListeners.delete(type);
      },
    });

    const sent: Array<{ kind: string; metric?: string; value_ms?: number; route?: string }> = [];
    const reporter = resetTelemetryReporterForTests({
      apiBaseUrl: "http://localhost:8000",
      fetchImpl: async (_url, init) => {
        const parsed = JSON.parse(typeof init?.body === "string" ? init.body : "{}") as {
          events: Array<{ kind: string; metric?: string; value_ms?: number; route?: string }>;
        };
        sent.push(...parsed.events);
        return new Response(null, { status: 202 });
      },
      isHidden: () => false,
      isOnline: () => true,
    });

    const uninstall = installBrowserTelemetry({
      apiBaseUrl: "http://localhost:8000",
      telemetryEnabled: true,
    });
    try {
      const findObserver = (type: string) =>
        observerRecords.find((item) => item.options.type === type);

      const lcpObs = findObserver("largest-contentful-paint");
      const navObs = findObserver("navigation");
      const eventObs = findObserver("event");
      expect(lcpObs).toBeDefined();
      expect(navObs).toBeDefined();
      expect(eventObs).toBeDefined();
      expect(eventObs?.options.durationThreshold).toBe(16);

      lcpObs?.callback(
        {
          getEntries: () => [{ startTime: 820 } as PerformanceEntry],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );
      navObs?.callback(
        {
          getEntries: () => [
            {
              requestStart: 120,
              responseStart: 350,
              activationStart: 50,
            } as unknown as PerformanceNavigationTiming,
          ],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );
      eventObs?.callback(
        {
          getEntries: () => [{ duration: 48, interactionId: 7 } as unknown as PerformanceEntry],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );

      // LCP 確定後に SPA 遷移しても記録時ルート (/channels) を維持する
      locationObj.pathname = "/home";
      windowListeners.get("pagehide")?.(new Event("pagehide"));

      await reporter.flush({ keepalive: true });
      const lcp = sent.find((e) => e.metric === "lcp");
      const ttfb = sent.find((e) => e.metric === "ttfb");
      const inp = sent.find((e) => e.metric === "inp");

      expect(lcp).toMatchObject({ metric: "lcp", value_ms: 820, route: "/channels" });
      expect(ttfb).toMatchObject({ metric: "ttfb", value_ms: 300, route: "/channels" });
      expect(inp).toMatchObject({ metric: "inp", value_ms: 48, route: "/channels" });
    } finally {
      uninstall();
      reporter.dispose();
    }
  });

  it("computes INP p98 using total interactionCount, preserves selected interaction route, and retries dedup-suppressed INP updates", async () => {
    let hidden = false;
    let now = 1000;
    const windowListeners = new Map<string, EventListener>();
    const documentListeners = new Map<string, EventListener>();
    const locationObj = { origin: "http://localhost:5173", pathname: "/home" };
    const perfMock = { interactionCount: 100 };
    const callbacks = new Map<string, PerformanceObserverCallback>();

    class MockPerformanceObserver {
      private callback: PerformanceObserverCallback;
      constructor(callback: PerformanceObserverCallback) {
        this.callback = callback;
      }
      observe(options: PerformanceObserverInit) {
        if (options.type) {
          callbacks.set(options.type, this.callback);
        }
      }
      disconnect() {}
    }

    vi.stubGlobal("PerformanceObserver", MockPerformanceObserver);
    vi.stubGlobal("performance", perfMock);
    vi.stubGlobal("window", {
      location: locationObj,
      addEventListener: (type: string, cb: EventListener) => {
        windowListeners.set(type, cb);
      },
      removeEventListener: (type: string) => {
        windowListeners.delete(type);
      },
    });
    vi.stubGlobal("document", {
      get visibilityState() {
        return hidden ? "hidden" : "visible";
      },
      addEventListener: (type: string, cb: EventListener) => {
        documentListeners.set(type, cb);
      },
      removeEventListener: (type: string) => {
        documentListeners.delete(type);
      },
    });

    const sent: Array<{ kind: string; metric?: string; value_ms?: number; route?: string }> = [];
    const reporter = resetTelemetryReporterForTests({
      apiBaseUrl: "http://localhost:8000",
      dedupWindowMs: 10_000,
      now: () => now,
      fetchImpl: async (_url, init) => {
        const parsed = JSON.parse(typeof init?.body === "string" ? init.body : "{}") as {
          events: Array<{ kind: string; metric?: string; value_ms?: number; route?: string }>;
        };
        sent.push(...parsed.events);
        return new Response(null, { status: 202 });
      },
      isHidden: () => hidden,
      isOnline: () => true,
    });

    const uninstall = installBrowserTelemetry({
      apiBaseUrl: "http://localhost:8000",
      telemetryEnabled: true,
    });
    try {
      const eventCb = callbacks.get("event");
      expect(eventCb).toBeDefined();

      // 1) 総操作数 100 で 3 件 (300, 200, 100) -> P98 順位で 3 番目の 100ms (/home)
      eventCb?.(
        {
          getEntries: () => [
            { duration: 300, interactionId: 1 } as unknown as PerformanceEntry,
            { duration: 200, interactionId: 2 } as unknown as PerformanceEntry,
            { duration: 100, interactionId: 3 } as unknown as PerformanceEntry,
          ],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );

      hidden = true;
      documentListeners.get("visibilitychange")?.(new Event("visibilitychange"));
      await Promise.resolve();
      expect(sent.map((e) => `${e.value_ms}@${e.route}`)).toEqual(["100@/home"]);

      // 2) 重複ウィンドウ内で /home に 500ms が発生（総数 2 扱いへ落とすため一旦少数ではなく追加悪化を確認）
      hidden = false;
      perfMock.interactionCount = 101;
      // interactionId 1,2 は 300,200 のまま 500ms が 2 件増えれば P98(101/50=2) が 300 -> 500ms テスト用に直接 4 件目として既存より遅い操作を追加:
      // 300/200/100 に加え id=4 (500ms), id=5 (400ms) にすると P98(102/50=2) は 3 番目の 300ms、
      // 102件中 3番目 500ms になるよう id=1,2,3 も 600,550,500 なら確実に選ばれる
      eventCb?.(
        {
          getEntries: () => [
            { duration: 600, interactionId: 1 } as unknown as PerformanceEntry,
            { duration: 550, interactionId: 2 } as unknown as PerformanceEntry,
            { duration: 500, interactionId: 3 } as unknown as PerformanceEntry,
          ],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );
      now += 5000;
      hidden = true;
      documentListeners.get("visibilitychange")?.(new Event("visibilitychange"));
      await Promise.resolve();
      // 10秒以内なので重複抑制で送られない
      expect(sent.map((e) => `${e.value_ms}@${e.route}`)).toEqual(["100@/home"]);

      // 3) 別画面 /channels で 32ms の操作が起きても、選ばれた 500ms のルートは /home のまま、
      // かつ 11秒後には重複抑制に捨てられた 500ms が送信される
      hidden = false;
      locationObj.pathname = "/channels/99";
      perfMock.interactionCount = 102;
      eventCb?.(
        {
          getEntries: () => [{ duration: 32, interactionId: 4 } as unknown as PerformanceEntry],
        } as unknown as PerformanceObserverEntryList,
        {} as PerformanceObserver,
      );
      now += 11_000;
      hidden = true;
      documentListeners.get("visibilitychange")?.(new Event("visibilitychange"));
      await Promise.resolve();
      expect(sent.map((e) => `${e.value_ms}@${e.route}`)).toEqual(["100@/home", "500@/home"]);
    } finally {
      uninstall();
      reporter.dispose();
    }
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

  it("reports body stream read failures inside the HTTP span", async () => {
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
      fetch: async () =>
        ({
          ok: true,
          status: 200,
          text: async () => {
            throw new Error("stream read failed");
          },
        }) as unknown as Response,
    });

    await expect(client.get("/api/channels")).rejects.toThrow("stream read failed");
    expect(reporter.pendingCount()).toBe(1);
    await reporter.flush();
    expect(telemetryCalls).toHaveLength(1);
    expect(telemetryCalls[0]).toContain('"category":"api"');
    expect(telemetryCalls[0]).toContain('"http_status":0');
    expect(telemetryCalls[0]).toContain("stream read failed");
    reporter.dispose();
  });

  it("records a single error when a 5xx API response fails while reading the body stream", async () => {
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
      fetch: async () =>
        ({
          ok: false,
          status: 503,
          text: async () => {
            throw new Error("body stream broken");
          },
        }) as unknown as Response,
    });

    await expect(client.get("/api/channels")).rejects.toThrow("body stream broken");
    expect(reporter.pendingCount()).toBe(1);
    await reporter.flush();
    expect(telemetryCalls).toHaveLength(1);
    expect(telemetryCalls[0]).toContain('"http_status":503');
    reporter.dispose();
  });

  it("reports Electric ReadableStream failures from wrapped 200 SSE responses", async () => {
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

    const failingStream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("data: {}\n\n"));
      },
      pull() {
        throw new Error("SSE stream dropped");
      },
    });

    const shapeFetch = createShapeFetch({
      getToken: () => null,
      apiBaseUrl: "http://localhost:8000",
      fetch: async () =>
        new Response(failingStream, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
    });

    const response = await shapeFetch("/api/shapes/messages?live=true");
    expect(response.status).toBe(200);
    await expect(response.text()).rejects.toThrow("SSE stream dropped");

    expect(reporter.pendingCount()).toBe(1);
    await reporter.flush();
    expect(telemetryCalls).toHaveLength(1);
    expect(telemetryCalls[0]).toContain('"kind":"electric_sync"');
    expect(telemetryCalls[0]).toContain('"shape":"messages"');
    expect(telemetryCalls[0]).toContain('"state":"error"');
    reporter.dispose();
  });

  it("respects VITE_TELEMETRY_ENABLED=false", () => {
    expect(resolveTelemetryEnabled(undefined)).toBe(true);
    expect(resolveTelemetryEnabled("false")).toBe(false);
    expect(resolvePublicConfig({ VITE_TELEMETRY_ENABLED: "false" }).telemetryEnabled).toBe(false);
  });
});
