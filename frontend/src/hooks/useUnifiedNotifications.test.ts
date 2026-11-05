// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { useUnifiedNotifications } from "./useUnifiedNotifications";
import type { NotificationRow } from "../lib/electric/types";
import type { ActivityPubNotification } from "./useActivityPubNotifications";
const mocks = vi.hoisted(() => ({
  localRead: vi.fn(),
  remoteRead: vi.fn(),
  rows: [] as NotificationRow[],
  items: [] as ActivityPubNotification[],
}));
vi.mock("./useNotifications", () => ({
  useNotifications: () => ({
    notifications: mocks.rows,
    unreadCount: 1,
    markRead: mocks.localRead,
  }),
}));
vi.mock("./useActivityPubNotifications", () => ({
  useActivityPubNotifications: () => ({
    items: mocks.items,
    unread_count: 2,
    markRead: mocks.remoteRead,
  }),
}));
it("uses one unread count and fans out selected or all reads to both stores without read-all for a selection", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.rows = [
    {
      id: "local",
      user_id: "me",
      actor_id: "alice",
      channel_id: "channel",
      message_id: "post",
      notification_type: "reaction",
      is_read: false,
      created_at: "2026-11-01T10:00:00Z",
    },
  ];
  const remote: ActivityPubNotification = {
    id: "mirror",
    actor_id: "local:alice",
    display_name: "Alice",
    kind: "like",
    activity_id: "activity",
    post_id: "post",
    content: null,
    is_read: false,
    pending: false,
    created_at: "2026-11-01T10:00:00Z",
  };
  mocks.items = [remote, { ...remote, id: "peer", actor_id: "https://peer/alice" }];
  mocks.localRead.mockResolvedValue(undefined);
  mocks.remoteRead.mockResolvedValue(undefined);
  let result!: ReturnType<typeof useUnifiedNotifications>;
  function Probe() {
    result = useUnifiedNotifications();
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(result.unreadCount).toBe(2);
    expect(result.entries).toHaveLength(2);
    await act(async () => result.markRead(["local", "activitypub:peer"]));
    expect(mocks.localRead).toHaveBeenLastCalledWith(["local"]);
    expect(mocks.remoteRead).toHaveBeenLastCalledWith(["peer", "mirror"]);
    await act(async () => result.markRead());
    expect(mocks.localRead).toHaveBeenLastCalledWith();
    expect(mocks.remoteRead).toHaveBeenLastCalledWith();
  } finally {
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
  }
});
