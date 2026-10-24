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
