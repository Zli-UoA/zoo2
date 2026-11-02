// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import {
  useActivityPubNotifications,
  type ActivityPubNotification,
} from "./useActivityPubNotifications";
const mocks = vi.hoisted(() => ({
  client: { get: vi.fn(), request: vi.fn() },
  config: {},
  live: undefined as undefined | (() => void),
}));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({ createConfiguredApiClient: () => mocks.client }));
vi.mock("./useHomeLive", () => ({
  useHomeLive: (callback: () => void) => {
    mocks.live = callback;
  },
}));
const item = (id: string, second: number, read = false): ActivityPubNotification => ({
  id,
  actor_id: "peer",
  display_name: "Peer",
  activity_id: id,
  kind: "like",
  post_id: "post",
  content: "post",
  pending: false,
  is_read: read,
  created_at: `2026-11-01T00:00:${String(second).padStart(2, "0")}Z`,
});
let root: ReturnType<typeof createRoot>;
let result: ReturnType<typeof useActivityPubNotifications>;
function Probe() {
  result = useActivityPubNotifications();
  return null;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.client.get.mockReset();
  mocks.client.request.mockReset();
  mocks.client.request.mockResolvedValue({ ok: true });
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
});
it("keeps loaded older pages after a refresh and acknowledges only the selected visible IDs", async () => {
  let read = false;
  mocks.client.get.mockImplementation(async (path: string) =>
    path.includes("before=")
      ? { items: [item("older", 1)], unread_count: read ? 2 : 3, next_cursor: null }
      : {
          items: [item("new", 3, read), item("middle", 2)],
          unread_count: read ? 2 : 3,
          next_cursor: "middle",
        },
  );
  await act(async () => root.render(createElement(Probe)));
  await act(async () => result.loadMore());
  expect(result.items.map((n) => n.id)).toEqual(["new", "middle", "older"]);
  mocks.client.request.mockImplementation(async () => {
    read = true;
    return {};
  });
  await act(async () => result.markRead(["new"]));
  expect(mocks.client.request).toHaveBeenCalledWith("/api/activitypub/notifications/read", {
    method: "POST",
    body: { ids: ["new"] },
  });
  await act(async () => result.refresh());
  expect(result.items.map((n) => n.id)).toEqual(["new", "middle", "older"]);
  expect(result.items.filter((n) => n.is_read).map((n) => n.id)).toEqual(["new"]);
  expect(result.unread_count).toBe(2);
});
it("defers a live refresh during older loading and retains its tail when a newer event arrives", async () => {
  let finish!: (value: unknown) => void;
  let newEvent = false;
  let olderCalls = 0;
  mocks.client.get.mockImplementation(async (path: string) => {
    if (!path.includes("before="))
      return {
        items: newEvent ? [item("arrived", 4), item("a", 3)] : [item("a", 3), item("b", 2)],
        unread_count: 4,
        next_cursor: "cursor",
      };
    if (++olderCalls === 1)
      return new Promise((resolve) => {
        finish = resolve;
      });
    return { items: [item("b", 2), item("c", 1)], unread_count: 4, next_cursor: null };
  });
  await act(async () => root.render(createElement(Probe)));
  let older!: Promise<void>;
  await act(async () => {
    older = result.loadMore();
  });
  newEvent = true;
  await act(async () => mocks.live?.());
  expect(mocks.client.get).toHaveBeenCalledTimes(2);
  await act(async () => {
    finish({ items: [item("c", 1)], unread_count: 4, next_cursor: null });
    await older;
  });
  await vi.waitFor(() => expect(result.items.map((n) => n.id)).toEqual(["arrived", "a", "b", "c"]));
  expect(result.next_cursor).toBeNull();
});
it("rolls back a rejected auto-read even if the recovery refresh is offline", async () => {
  mocks.client.get.mockResolvedValue({ items: [item("unread", 1)], unread_count: 4 });
  await act(async () => root.render(createElement(Probe)));
  mocks.client.request.mockRejectedValue(new Error("offline"));
  mocks.client.get.mockRejectedValue(new Error("offline"));
  await act(async () => {
    await expect(result.markRead(["unread"])).rejects.toThrow("offline");
  });
  expect(result.items[0].is_read).toBe(false);
  expect(result.unread_count).toBe(4);
  expect(result.error).toBe(true);
});
