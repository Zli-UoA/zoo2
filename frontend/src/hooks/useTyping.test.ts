// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { useTyping } from "./useTyping";

const mocks = vi.hoisted(() => ({ request: vi.fn(), get: vi.fn(), config: {} }));
vi.mock("../lib/electric/collections", () => ({
  getCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ request: mocks.request, get: mocks.get }),
}));
let root: Root;
let element: HTMLDivElement;
let typing: ReturnType<typeof useTyping>;
function Probe({
  channel = "channel",
  parent = null,
  enabled = true,
}: {
  channel?: string;
  parent?: string | null;
  enabled?: boolean;
}) {
  typing = useTyping(channel, parent, enabled);
  return null;
}
async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.request.mockReset().mockResolvedValue({ ok: true });
  mocks.get.mockReset().mockResolvedValue([{ id: "alice", display_name: "Alice" }]);
  element = document.createElement("div");
  document.body.append(element);
  root = createRoot(element);
});
afterEach(async () => {
  await flush(() => root.unmount());
  element.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
const payloads = () => mocks.request.mock.calls.map((call) => call[1].body);
describe("useTyping", () => {
  it("stops on hidden tabs and ignores delayed responses after leaving", async () => {
    let resolve!: (users: Array<{ id: string; display_name: string }>) => void;
    mocks.get.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    await flush(() => root.render(createElement(Probe)));
    await flush(() => typing.onTyping(true));
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    await flush(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(payloads().at(-1).active).toBe(false);
    await flush(() => resolve([{ id: "alice", display_name: "Alice" }]));
    expect(typing.users).toEqual([]);
    visibility.mockRestore();
  });

  it("throttles activity and stops after inactivity without sending drafts", async () => {
    await flush(() => root.render(createElement(Probe)));
    expect(typing.users[0].display_name).toBe("Alice");
    await flush(() => {
      typing.onTyping(true);
      typing.onTyping(true);
    });
    expect(payloads()).toEqual([{ parent_id: null, session_id: expect.any(String), active: true }]);
    await flush(() => vi.advanceTimersByTime(2000));
    await flush(() => typing.onTyping(true));
    expect(payloads()).toHaveLength(2);
    await flush(() => vi.advanceTimersByTime(3000));
    expect(payloads().at(-1).active).toBe(false);
  });
  it("isolates threads and stops on channel changes or disabling", async () => {
    await flush(() => root.render(createElement(Probe, { parent: "root" })));
    expect(mocks.get.mock.calls[0][0]).toBe("/api/channels/channel/typing?parent_id=root");
    await flush(() => typing.onTyping(true));
    const session = payloads()[0].session_id;
    await flush(() => root.render(createElement(Probe, { channel: "other" })));
    expect(payloads().at(-1)).toEqual({ parent_id: "root", session_id: session, active: false });
    await flush(() => root.render(createElement(Probe, { enabled: false })));
    expect(typing.users).toEqual([]);
    const count = mocks.request.mock.calls.length;
    await flush(() => typing.onTyping(true));
    expect(mocks.request).toHaveBeenCalledTimes(count);
  });
  it("clears on explicit stop, unmount, and polling failure", async () => {
    await flush(() => root.render(createElement(Probe)));
    await flush(() => typing.onTyping(true));
    await flush(() => typing.onTyping(false));
    expect(payloads().at(-1).active).toBe(false);
    mocks.get.mockRejectedValue(new Error("offline"));
    await flush(() => vi.advanceTimersByTime(1500));
    expect(typing.users).toEqual([]);
    await flush(() => typing.onTyping(true));
    await flush(() => root.render(null));
    expect(payloads().at(-1).active).toBe(false);
  });
});
