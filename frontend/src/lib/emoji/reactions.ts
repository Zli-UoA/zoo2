import type { MessageReactionRow } from "../electric/types";
import { reactionKey } from "./kitchen";

export interface ReactionGroup {
  emoji: string;
  count: number;
  ownId: string | null;
}

export function groupReactions(
  rows: MessageReactionRow[],
  currentUserId: string | null,
): Map<string, ReactionGroup[]> {
  const messages = new Map<
    string,
    Map<string, { emoji: string; users: Set<string>; ownId: string | null }>
  >();
  for (const row of rows) {
    let groups = messages.get(row.message_id);
    if (!groups) {
      groups = new Map();
      messages.set(row.message_id, groups);
    }
    const key = reactionKey(row.emoji);
    let group = groups.get(key);
    if (!group) {
      group = { emoji: row.emoji, users: new Set(), ownId: null };
      groups.set(key, group);
    }
    group.users.add(row.user_id);
    if (row.user_id === currentUserId) group.ownId = row.id;
  }
  return new Map(
    Array.from(messages, ([messageId, groups]) => [
      messageId,
      Array.from(groups.values(), ({ emoji, users, ownId }) => ({
        emoji,
        count: users.size,
        ownId,
      })),
    ]),
  );
}
