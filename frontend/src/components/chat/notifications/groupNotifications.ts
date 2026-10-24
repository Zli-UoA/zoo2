import type { NotificationRow } from "../../../lib/electric/types";
import { parseUtcTimestamp } from "../../../lib/timestamps";

export interface NotificationGroup {
  key: string;
  latest: NotificationRow;
  notifications: NotificationRow[];
  actorIds: string[];
}

/** Reactions to one message share a card; mentions and replies remain separate events. */
export function groupNotifications(rows: readonly NotificationRow[]): NotificationGroup[] {
  const groups = new Map<string, NotificationGroup>();
  const sorted = [...rows].sort(
    (a, b) =>
      parseUtcTimestamp(b.created_at).getTime() - parseUtcTimestamp(a.created_at).getTime() ||
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
