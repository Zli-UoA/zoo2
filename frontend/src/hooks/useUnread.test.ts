// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { READ_STATE_CHANGED, UnreadProvider, useUnread } from "./useUnread";

const api = vi.hoisted(() => ({ get: vi.fn() }));
const user = vi.hoisted(() => ({ id: "alice" }));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => api }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: { apiBaseUrl: "", devUserId: user.id } }),
}));
vi.mock("./useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: user.id }),
}));

const state = (channelId: string, first: string | null, count = 1) => ({
  channel_id: channelId,
  last_read_at: null,
  has_unread: true,
  unread_count: count,
  unread_count_truncated: false,
  first_unread_at: first,
});

it("shares one fetch, orders by oldest unread and reflects local reads", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  api.get.mockReset();
  const root = createRoot(document.createElement("div"));
  let value!: ReturnType<typeof useUnread>;
  function Test() {
    value = useUnread();
    return null;
  }
  api.get.mockResolvedValue([state("newer", "2026-01-02"), state("older", "2026-01-01", 3)]);
  try {
    await act(async () => root.render(createElement(UnreadProvider, null, createElement(Test))));
    expect(api.get).toHaveBeenCalledTimes(1);
    expect(value.unreadStates.map((s) => s.channel_id)).toEqual(["older", "newer"]);
    expect(value.unreadConversationCount).toBe(2);

    // Concurrent notifications collapse into one trailing fetch.
    let release!: (value: unknown) => void;
    api.get.mockImplementation(() => new Promise((resolve) => (release = resolve)));
    await act(async () => {
      window.dispatchEvent(new Event(READ_STATE_CHANGED));
    });
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(api.get).toHaveBeenCalledTimes(2);
    api.get.mockResolvedValue([state("fourth", "2026-01-04")]);
    await act(async () => {
      release([state("third", "2026-01-03")]);
      await Promise.resolve();
    });
    expect(api.get).toHaveBeenCalledTimes(3);
    expect(value.unreadStates.map((s) => s.channel_id)).toEqual(["fourth"]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });
    expect(api.get).toHaveBeenCalledTimes(4);

    await act(async () => {
      value.applyRead({ ...state("fourth", "2026-01-04"), has_unread: false });
    });
    expect(value.unreadConversationCount).toBe(0);

    api.get.mockImplementation(() => new Promise(() => {}));
    user.id = "bob";
    await act(async () => root.render(createElement(UnreadProvider, null, createElement(Test))));
    expect(value.unreadConversationCount).toBe(0);
  } finally {
    await act(async () => root.unmount());
    user.id = "alice";
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});

it("does not fetch while the tab is hidden and refreshes on return", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  api.get.mockReset();
  const root = createRoot(document.createElement("div"));
  function Test() {
    useUnread();
    return null;
  }
  api.get.mockResolvedValue([]);
  const visibility = vi
    .spyOn(document, "visibilityState", "get")
    .mockReturnValue("hidden" as DocumentVisibilityState);
  try {
    await act(async () => root.render(createElement(UnreadProvider, null, createElement(Test))));
    expect(api.get).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30000);
    });
    expect(api.get).not.toHaveBeenCalled();
    visibility.mockReturnValue("visible");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(api.get).toHaveBeenCalledTimes(1);
  } finally {
    await act(async () => root.unmount());
    visibility.mockRestore();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});
