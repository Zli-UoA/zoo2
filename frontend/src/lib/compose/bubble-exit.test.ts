import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { createBubbleExit } from "./bubble-exit";

describe("native Bubble exit transport", () => {
  let lifecycle: EventTarget;
  let visibility: EventTarget & { visibilityState: string };
  let transport: ReturnType<typeof createBubbleExit>;
  const fetchMock = vi.fn();
  const update = (session = "current", seq = 5) =>
    transport.update(
      JSON.stringify({
        session,
        offset: 1000,
        body: { session_id: session, seq, content: "" },
        headers: { "X-Dev-User": "user-1" },
      }),
    );

  beforeEach(() => {
    lifecycle = new EventTarget();
    visibility = Object.assign(new EventTarget(), { visibilityState: "visible" });
    vi.stubGlobal("window", lifecycle);
    vi.stubGlobal("document", visibility);
    fetchMock.mockReset().mockResolvedValue(new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(Date, "now").mockReturnValue(10000);
    transport = createBubbleExit("https://api.test/api/bubble");
  });
  afterEach(() => {
    transport.close();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("starts the keepalive synchronously before pagehide and sends each snapshot once", () => {
    update();
    lifecycle.dispatchEvent(new Event("beforeunload"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url).toBe("https://api.test/api/bubble");
    expect(options.keepalive).toBe(true);
    expect(options.headers.get("X-Dev-User")).toBe("user-1");
    expect(JSON.parse(options.body)).toEqual({
      session_id: "current",
      seq: 5,
      content: "",
      sent_at: 11000,
    });
    visibility.visibilityState = "hidden";
    visibility.dispatchEvent(new Event("visibilitychange"));
    lifecycle.dispatchEvent(new Event("pagehide"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("also handles pagehide without beforeunload", () => {
    update();
    lifecycle.dispatchEvent(new Event("pagehide"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("clears only when visibility becomes hidden", () => {
    update();
    visibility.dispatchEvent(new Event("visibilitychange"));
    expect(fetchMock).not.toHaveBeenCalled();
    visibility.visibilityState = "hidden";
    visibility.dispatchEvent(new Event("visibilitychange"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("keeps a newer session when an older controller completes cleanup", () => {
    update("old");
    update("new", 8);
    transport.update(JSON.stringify({ session: "old", clear: "true" }));
    lifecycle.dispatchEvent(new Event("beforeunload"));
    expect(JSON.parse(fetchMock.mock.calls[0][1].body).session_id).toBe("new");
    update("new", 9);
    transport.update(JSON.stringify({ session: "new", clear: "true" }));
    lifecycle.dispatchEvent(new Event("pagehide"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("clears during disposal and ignores late controller writes and lifecycle events", () => {
    update();
    transport.close();
    update("late");
    lifecycle.dispatchEvent(new Event("beforeunload"));
    lifecycle.dispatchEvent(new Event("pagehide"));
    transport.close();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
