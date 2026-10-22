// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { READ_STATE_CHANGED, useUnreadChannels } from "./useUnreadChannels";

const api = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../lib/electric/collections", () => ({ getCollections: () => ({ config: {} }) }));

it("refreshes unread channels periodically and immediately after a read, and clears on user switch", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  const root = createRoot(document.createElement("div"));
  let unread!: Set<string>;
  function Test({ userId }: { userId: string | null }) {
    unread = useUnreadChannels(userId);
    return null;
  }
  api.get.mockResolvedValue([
    { channel_id: "one", has_unread: true },
    { channel_id: "two", has_unread: false },
  ]);
  try {
    await act(async () => root.render(createElement(Test, { userId: "alice" })));
    expect([...unread]).toEqual(["one"]);
    api.get.mockResolvedValue([{ channel_id: "one", has_unread: false }]);
    await act(async () => {
      window.dispatchEvent(new Event(READ_STATE_CHANGED));
    });
    expect(unread.size).toBe(0);
    api.get.mockResolvedValue([{ channel_id: "two", has_unread: true }]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });
    expect([...unread]).toEqual(["two"]);
    api.get.mockImplementation(() => new Promise(() => {}));
    await act(async () => root.render(createElement(Test, { userId: "bob" })));
    expect(unread.size).toBe(0);
    await act(async () => root.render(createElement(Test, { userId: null })));
    expect(unread.size).toBe(0);
  } finally {
    await act(async () => root.unmount());
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});
