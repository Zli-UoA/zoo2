// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { NotificationsProvider, useUnifiedNotifications } from "./useUnifiedNotifications";
import type { ActivityPubNotification } from "./useActivityPubNotifications";

const mocks = vi.hoisted(() => ({
  config: { apiBaseUrl: "https://zoo.test", devUserId: "me" },
  userId: "me",
  get: vi.fn(),
  request: vi.fn(),
}));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ get: mocks.get, request: mocks.request }),
}));
vi.mock("./useCurrentUser", () => ({ useCurrentUser: () => ({ currentUserId: mocks.userId }) }));
vi.mock("./useNotifications", () => ({
  useNotifications: () => ({ notifications: [], unreadCount: 0, markRead: vi.fn() }),
}));
vi.mock("./useHomeLive", () => ({ useHomeLive: () => {} }));
const row: ActivityPubNotification = {
  id: "notification",
  actor_id: "peer",
  display_name: "Peer",
  kind: "like",
  activity_id: "activity",
  post_id: "post",
  content: null,
  pending: false,
  is_read: false,
  created_at: "2026-11-01T00:00:00Z",
};
let root: ReturnType<typeof createRoot>;
const results: Record<string, ReturnType<typeof useUnifiedNotifications>> = {};
function Probe({ name }: { name: string }) {
  results[name] = useUnifiedNotifications();
  return null;
}
const tree = (screen: boolean) =>
  createElement(NotificationsProvider, {
    children: [
      createElement(Probe, { key: "badge", name: "badge" }),
      screen ? createElement(Probe, { key: "screen", name: "screen" }) : null,
    ],
  });
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.userId = "me";
  mocks.get.mockReset().mockResolvedValue({ items: [row], unread_count: 1 });
  mocks.request.mockReset().mockResolvedValue({});
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
});
it("shares fetches, pagination and optimistic reads between badge and screen across navigation", async () => {
  mocks.get.mockResolvedValueOnce({ items: [row], unread_count: 2, next_cursor: "next" });
  await act(async () => root.render(tree(false)));
  expect(mocks.get).toHaveBeenCalledTimes(1);
  await act(async () => root.render(tree(true)));
  expect(mocks.get).toHaveBeenCalledTimes(1);
  expect(results.badge).toBe(results.screen);
  mocks.get.mockResolvedValueOnce({
    items: [{ ...row, id: "older" }],
    unread_count: 2,
    next_cursor: null,
  });
  await act(async () => results.screen.activitypub.loadMore());
  expect(results.badge.activitypub.items.map((item) => item.id)).toEqual(["notification", "older"]);
  let finish!: () => void;
  mocks.request.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  let read!: Promise<void>;
  await act(async () => {
    read = results.screen.markRead(["activitypub:notification"]);
  });
  expect(results.badge.unreadCount).toBe(1);
  expect(results.screen.unreadCount).toBe(1);
  await act(async () => {
    finish();
    await read;
  });
  await act(async () => root.render(tree(false)));
  await act(async () => root.render(tree(true)));
  expect(mocks.get).toHaveBeenCalledTimes(2);
  expect(results.screen.activitypub.items).toHaveLength(2);
});
it("clears the old account and aborts its pending request on account switch", async () => {
  let signal!: AbortSignal;
  mocks.get
    .mockImplementationOnce((_path: string, options: { signal: AbortSignal }) => {
      signal = options.signal;
      return new Promise((_resolve, reject) =>
        signal.addEventListener("abort", () => reject(signal.reason), { once: true }),
      );
    })
    .mockResolvedValueOnce({ items: [], unread_count: 0 });
  await act(async () => root.render(tree(true)));
  mocks.userId = "other";
  await act(async () => root.render(tree(true)));
  expect(signal.aborted).toBe(true);
  expect(mocks.get).toHaveBeenCalledTimes(2);
  expect(results.badge.activitypub.items).toEqual([]);
  expect(results.badge.unreadCount).toBe(0);
});
