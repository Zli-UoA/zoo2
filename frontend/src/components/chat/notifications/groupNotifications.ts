import { reactionKey } from "../../../lib/emoji/kitchen";
import type { NotificationRow } from "../../../lib/electric/types";
import { parseUtcTimestamp } from "../../../lib/timestamps";

export interface NotificationGroup {
  key: string;
  latest: NotificationRow;
  notifications: NotificationRow[];
  actorIds: string[];
}

/** Preserve server microseconds when interleaving notifications from both sources. */
export function notificationTimestamp(value: string): number {
  const fraction = /[T ]\d{2}:\d{2}:\d{2}\.(\d+)/.exec(value)?.[1] ?? "";
  return parseUtcTimestamp(value).getTime() * 1000 + Number(fraction.padEnd(6, "0").slice(3, 6));
}

/** Reactions to one message share a card; mentions and replies remain separate events. */
export function groupNotifications(rows: readonly NotificationRow[]): NotificationGroup[] {
  const groups = new Map<string, NotificationGroup>();
  const sorted = [...rows].sort(
    (a, b) =>
      notificationTimestamp(b.created_at) - notificationTimestamp(a.created_at) ||
      b.id.localeCompare(a.id),
  );
  for (const row of sorted) {
    const key =
      row.notification_type === "reaction"
        ? `reaction:${row.user_id}:${row.channel_id}:${row.message_id}`
        : `notification:${row.id}`;
    const group = groups.get(key);
    if (group) {
      group.notifications.push(row);
      if (!group.actorIds.includes(row.actor_id)) group.actorIds.push(row.actor_id);
    } else {
      groups.set(key, { key, latest: row, notifications: [row], actorIds: [row.actor_id] });
    }
  }
  return [...groups.values()];
}

export function quotePreview(content: string): string {
  const characters = Array.from(content);
  return characters.length > 280 ? `${characters.slice(0, 280).join("")}…` : content;
}

export function notificationReactionGroups(
  rows: readonly { user_id: string; emoji: string }[],
  actorIds: readonly string[],
): { emoji: string; count: number }[] {
  const actors = new Set(actorIds);
  const groups = new Map<string, { emoji: string; users: Set<string> }>();
  for (const row of rows) {
    if (!actors.has(row.user_id)) continue;
    const key = reactionKey(row.emoji);
    let group = groups.get(key);
    if (!group) {
      group = { emoji: row.emoji, users: new Set() };
      groups.set(key, group);
    }
    group.users.add(row.user_id);
  }
  return [...groups.values()].map(({ emoji, users }) => ({ emoji, count: users.size }));
}

export type FederatedNotification =
  import("../../../hooks/useActivityPubNotifications").ActivityPubNotification;
export type UnifiedNotificationEntry =
  | {
      source: "channel";
      key: string;
      created_at: string;
      group: NotificationGroup;
      mirrors: { channelId: string; item: FederatedNotification }[];
    }
  | { source: "activitypub"; key: string; created_at: string; item: FederatedNotification };

/** One chronological feed. Legacy cross-post notifications acknowledge both source records. */
export function unifiedNotificationFeed(
  rows: readonly NotificationRow[],
  federated: readonly FederatedNotification[],
  hasOlderFederated = false,
) {
  const channels: Extract<UnifiedNotificationEntry, { source: "channel" }>[] = groupNotifications(
    rows,
  ).map((group) => ({
    source: "channel",
    key: `channel:${group.key}`,
    created_at: group.latest.created_at,
    group,
    mirrors: [],
  }));
  const matches = new Map<string, { entry: (typeof channels)[number]; row: NotificationRow }>();
  for (const entry of channels)
    for (const row of entry.group.notifications) {
      const kind =
        row.notification_type === "reaction"
          ? "like"
          : ["mention", "reply"].includes(row.notification_type)
            ? "reply"
            : null;
      if (kind) matches.set(`${kind}:${row.actor_id}:${row.message_id}`, { entry, row });
    }
  const entries: UnifiedNotificationEntry[] = [...channels];
  let duplicateUnreadCount = 0;
  for (const item of federated) {
    const match =
      item.actor_id.startsWith("local:") && item.post_id
        ? matches.get(`${item.kind}:${item.actor_id.slice(6)}:${item.post_id}`)
        : undefined;
    if (match) {
      match.entry.mirrors.push({ channelId: match.row.id, item });
      if (!item.is_read && !match.row.is_read) duplicateUnreadCount++;
    } else
      entries.push({
        source: "activitypub",
        key: `activitypub:${item.id}`,
        created_at: item.created_at,
        item,
      });
  }
  for (const entry of channels)
    if (entry.mirrors.some(({ item }) => !item.is_read)) {
      entry.group = {
        ...entry.group,
        notifications: entry.group.notifications.map((row) => ({
          ...row,
          is_read:
            row.is_read &&
            !entry.mirrors.some((mirror) => mirror.channelId === row.id && !mirror.item.is_read),
        })),
      };
    }
  const cutoff =
    hasOlderFederated && federated.length
      ? federated.reduce(
          (oldest, item) => Math.min(oldest, notificationTimestamp(item.created_at)),
          Infinity,
        )
      : -Infinity;
  return {
    entries: entries
      .filter((entry) => notificationTimestamp(entry.created_at) >= cutoff)
      .sort(
        (a, b) =>
          notificationTimestamp(b.created_at) - notificationTimestamp(a.created_at) ||
          b.key.localeCompare(a.key),
      ),
    // Keep all mirrors for unread counts and acknowledgement, even beyond the loaded frontier.
    mirrors: channels.flatMap((entry) => entry.mirrors),
    duplicateUnreadCount,
  };
}
