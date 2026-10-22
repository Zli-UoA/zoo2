// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vite-plus/test";

import { resetTokenProvider, setTokenProvider } from "../auth";
import { resolveConfig } from "../config";
import { getCollections } from "./collections";

afterEach(() => {
  resetTokenProvider();
  vi.unstubAllGlobals();
});

it.each(["direct", "proxy"] as const)("receives live updates over SSE in %s mode", async (mode) => {
  setTokenProvider(() => "test-token");
  const requests: { url: URL; headers: Headers }[] = [];
  let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
  const headers = {
    "electric-handle": "test-handle",
    "electric-offset": "0_0",
    "electric-cursor": "test-cursor",
    "electric-schema": JSON.stringify({
      id: { type: "text" },
      user_name: { type: "text" },
      display_name: { type: "varchar" },
      created_at: { type: "timestamp" },
      updated_at: { type: "timestamp" },
    }),
  };
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (input, init) => {
      const url = new URL(input instanceof Request ? input.url : input);
      if (url.pathname.endsWith("/me")) {
        return new Response(JSON.stringify({ id: "dev-user" }), {
          headers: { "content-type": "application/json" },
        });
      }
      requests.push({ url, headers: new Headers(init?.headers) });
      if (url.searchParams.get("live") === "true") {
        return new Response(
          new ReadableStream<Uint8Array>({
            start(value) {
              controller = value;
            },
          }),
          { headers: { ...headers, "content-type": "text/event-stream" } },
        );
      }
      return new Response(JSON.stringify([{ headers: { control: "up-to-date" } }]), {
        headers: { ...headers, "content-type": "application/json" },
      });
    }),
  );
  const bundle = getCollections(
    resolveConfig({ DEV: true, VITE_SHAPE_MODE: mode, VITE_DEV_USER_ID: "dev-user" }),
  );
  try {
    await bundle.users.preload();
    await vi.waitFor(() => expect(controller).toBeDefined());
    const requestCount = requests.length;
    expect(requests[0].url.searchParams.get("offset")).toBe("-1");
    const live = requests.find((request) => request.url.searchParams.get("live") === "true")!;
    expect(live.url.searchParams.get("live_sse")).toBe("true");
    expect(live.url.searchParams.get("handle")).toBe("test-handle");
    expect(live.headers.get("accept")).toBe("text/event-stream");
    expect(live.headers.get("authorization")).toBe(mode === "proxy" ? "Bearer test-token" : null);
    expect(live.headers.get("X-Dev-User")).toBe(mode === "proxy" ? "dev-user" : null);
    const user = {
      id: "user-1",
      user_name: "alice",
      display_name: "Alice",
      created_at: "2026-10-23 00:00:00",
      updated_at: null,
    };
    const events = [
      { key: '"users"/"user-1"', value: user, headers: { operation: "insert" } },
      { headers: { control: "up-to-date", global_last_seen_lsn: "1" } },
    ];
    controller!.enqueue(
      new TextEncoder().encode(
        events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
      ),
    );
    await vi.waitFor(() => expect(bundle.users.get("user-1")?.display_name).toBe("Alice"));
    // Updates arrive on the open stream, without another HTTP request.
    expect(requests).toHaveLength(requestCount);
  } finally {
    await Promise.all([
      bundle.users.cleanup(),
      bundle.channels.cleanup(),
      bundle.channelUsers.cleanup(),
    ]);
    controller?.close();
  }
});
