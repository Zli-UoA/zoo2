import { expect, it } from "vite-plus/test";
import type { NotificationRow } from "../../../lib/electric/types";
import { groupNotifications, quotePreview, notificationReactionGroups } from "./groupNotifications";

const row = (id: string, overrides: Partial<NotificationRow> = {}): NotificationRow => ({
  id,
  user_id: "me",
  actor_id: "alice",
  channel_id: "channel",
  message_id: "message",
  notification_type: "reaction",
  is_read: false,
  created_at: "2026-10-24T01:00:00Z",
  ...overrides,
});
it("groups reactions to a message, deduplicates actors and keeps mentions and replies separate", () => {
  const rows = [
    row("old", { is_read: true }),
    row("bob", { actor_id: "bob", created_at: "2026-10-24 02:00:00" }),
    row("again"),
    row("mention", { notification_type: "mention" }),
    row("reply", { notification_type: "reply" }),
    row("other-message", { message_id: "another" }),
    row("other-channel", { channel_id: "another" }),
    row("other-user", { user_id: "another" }),
  ];
  const groups = groupNotifications(rows);
  expect(groups).toHaveLength(6);
  expect(groups[0].latest.id).toBe("bob");
  expect(groups[0].actorIds).toEqual(["bob", "alice"]);
  expect(groups[0].notifications).toHaveLength(3);
  expect(groups[0].notifications.filter((n) => !n.is_read).map((n) => n.id)).toEqual([
    "bob",
    "again",
  ]);
  expect(rows[0].id).toBe("old");
});
it("keeps group identity stable after reads and moves a group up when a new reaction arrives", () => {
  const first = groupNotifications([
    row("a"),
    row("b", { message_id: "other", created_at: "2026-10-24T02:00:00Z" }),
  ]);
  const next = groupNotifications([
    row("a", { is_read: true }),
    row("new", { created_at: "2026-10-24T03:00:00Z" }),
    row("b", { message_id: "other", created_at: "2026-10-24T02:00:00Z" }),
  ]);
  expect(next[0].key).toBe(first[1].key);
  expect(next[0].notifications.map((n) => n.id)).toEqual(["new", "a"]);
});
it("limits long previews without splitting emoji and preserves short multiline quotations", () => {
  expect(quotePreview("First\nSecond")).toBe("First\nSecond");
  const long = "🦊".repeat(300);
  expect(quotePreview(long)).toBe("🦊".repeat(280) + "…");
  expect(quotePreview("a".repeat(280))).toHaveLength(280);
});

it("counts only notification actors and deduplicates equivalent kitchen reactions", () => {
  expect(
    notificationReactionGroups(
      [
        { user_id: "alice", emoji: "👍" },
        { user_id: "alice", emoji: "👍" },
        { user_id: "outsider", emoji: "👍" },
        { user_id: "bob", emoji: "👍" },
        { user_id: "alice", emoji: "kitchen:1f600_1f431" },
        { user_id: "bob", emoji: "kitchen:1f431_1f600" },
      ],
      ["alice", "bob"],
    ),
  ).toEqual([
    { emoji: "👍", count: 2 },
    { emoji: "kitchen:1f600_1f431", count: 2 },
  ]);
});

import {
  unifiedNotificationFeed,
  notificationTimestamp,
  type FederatedNotification,
} from "./groupNotifications";
const remote = (
  id: string,
  overrides: Partial<FederatedNotification> = {},
): FederatedNotification => ({
  id,
  actor_id: "https://peer.example/alice",
  display_name: "Remote Alice",
  kind: "like",
  activity_id: `activity:${id}`,
  post_id: "message",
  content: "post",
  is_read: false,
  pending: false,
  created_at: "2026-10-24T01:00:00Z",
  reaction: "🦊",
  ...overrides,
});
it("interleaves channel and federated events by time, with stable ordering after reads and pending decisions", () => {
  const local = [row("local", { created_at: "2026-10-24T01:00:00.000002Z" })];
  const apub = [
    remote("new", { created_at: "2026-10-24T01:00:00.000003Z" }),
    remote("old", {
      pending: true,
      kind: "follow_request",
      created_at: "2026-10-24T01:00:00.000001Z",
    }),
  ];
  const feed = unifiedNotificationFeed(local, apub);
  expect(feed.entries.map((entry) => entry.key)).toEqual([
    "activitypub:new",
    "channel:reaction:me:channel:message",
    "activitypub:old",
  ]);
  expect(
    unifiedNotificationFeed(
      local.map((n) => ({ ...n, is_read: true })),
      apub.map((n) => ({ ...n, is_read: true, pending: false })),
    ).entries.map((entry) => entry.key),
  ).toEqual(feed.entries.map((entry) => entry.key));
  expect(notificationTimestamp("2026-10-24T10:00:00.000002+09:00")).toBe(
    notificationTimestamp(local[0].created_at),
  );
});
it("merges local cross-post duplicates and retains both read identities without changing source rows", () => {
  const local = [row("local")];
  const feed = unifiedNotificationFeed(local, [
    remote("mirror", { actor_id: "local:alice" }),
    remote("external"),
  ]);
  expect(feed.entries).toHaveLength(2);
  expect(feed.mirrors.map((mirror) => [mirror.channelId, mirror.item.id])).toEqual([
    ["local", "mirror"],
  ]);
  expect(feed.duplicateUnreadCount).toBe(1);
  const alreadyRead = unifiedNotificationFeed(
    [row("local", { is_read: true })],
    [remote("mirror", { actor_id: "local:alice" })],
  );
  expect(
    alreadyRead.entries[0].source === "channel" &&
      alreadyRead.entries[0].group.notifications[0].is_read,
  ).toBe(false);
  expect(alreadyRead.duplicateUnreadCount).toBe(0);
  expect(local[0].is_read).toBe(false);
});
it("holds older channel events behind the federated paging frontier until its next page is loaded", () => {
  const local = [
    row("old", { created_at: "2026-10-23T00:00:00Z" }),
    row("new", { notification_type: "mention", created_at: "2026-10-25T00:00:00Z" }),
  ];
  expect(
    unifiedNotificationFeed(local, [remote("remote")], true).entries.map((n) => n.key),
  ).toEqual(["channel:notification:new", "activitypub:remote"]);
  expect(unifiedNotificationFeed(local, [remote("remote")], false).entries).toHaveLength(3);
});
