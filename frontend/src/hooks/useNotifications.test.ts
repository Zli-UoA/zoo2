// @vitest-environment happy-dom
import { createCollection, localOnlyCollectionOptions } from "@tanstack/react-db";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import type { NotificationRow } from "../lib/electric/types";
import { useNotifications } from "./useNotifications";

const mocks = vi.hoisted(() => ({ userId: "alice" as string | null, notifications: vi.fn() }));
vi.mock("./useCurrentUser", () => ({ useCurrentUser: () => ({ currentUserId: mocks.userId }) }));
vi.mock("../lib/electric/collections", () => ({
  getCollections: () => ({ notifications: mocks.notifications }),
}));

it("counts live unread notifications, persists read changes and clears data on user switch", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const notification = (id: string, userId: string, read = false): NotificationRow => ({
    id,
    user_id: userId,
    actor_id: "actor",
    channel_id: "channel",
    message_id: "message",
    notification_type: "mention",
    is_read: read,
    created_at: "2026-10-24T00:00:00Z",
  });
  const alice = createCollection(
    localOnlyCollectionOptions<NotificationRow>({
      getKey: (row) => row.id,
      initialData: [
        notification("n1", "alice"),
        notification("n2", "alice", true),
        notification("n3", "bob"),
      ],
    }),
  );
  const bob = createCollection(
    localOnlyCollectionOptions<NotificationRow>({
      getKey: (row) => row.id,
      initialData: [notification("n3", "bob")],
    }),
  );
  mocks.notifications.mockImplementation((userId: string) => (userId === "alice" ? alice : bob));
  const root = createRoot(document.createElement("div"));
  let result!: ReturnType<typeof useNotifications>;
  function Probe() {
    result = useNotifications();
    return null;
  }
  try {
    await act(async () => root.render(createElement(Probe)));
    await vi.waitFor(() => expect(result.unreadCount).toBe(1));
    expect(result.notifications.map((row) => row.id)).toEqual(["n2", "n1"]);
    await act(async () => {
      await result.markRead("n1");
    });
    expect(result.unreadCount).toBe(0);
    expect(alice.get("n1")?.is_read).toBe(true);
    await act(async () => {
      alice.insert(notification("n4", "alice"));
    });
    expect(result.unreadCount).toBe(1);
    await act(async () => {
      alice.insert(notification("offscreen", "alice"));
    });
    await act(async () => {
      await result.markRead(["n4"]);
    });
    expect(alice.get("n4")?.is_read).toBe(true);
    expect(alice.get("offscreen")?.is_read).toBe(false);
    expect(result.unreadCount).toBe(1);
    await act(async () => {
      await result.markRead();
    });
    expect(result.unreadCount).toBe(0);
    mocks.userId = "bob";
    await act(async () => root.render(createElement(Probe)));
    await vi.waitFor(() => expect(result.notifications.map((row) => row.id)).toEqual(["n3"]));
    expect(result.unreadCount).toBe(1);
    mocks.userId = null;
    await act(async () => root.render(createElement(Probe)));
    expect(result.notifications).toEqual([]);
    expect(result.unreadCount).toBe(0);
  } finally {
    await act(async () => root.unmount());
    await alice.cleanup();
    await bob.cleanup();
    vi.unstubAllGlobals();
  }
});
