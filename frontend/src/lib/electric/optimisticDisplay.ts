import type { MessageRow, MessageReactionRow, MessageDisplayData } from "./types";

import type { WorkspaceEmoji } from "../emoji/workspace";

type Rows<T extends { id: string }> = Pick<ReadonlyMap<string | number, T>, "get" | "keys">;
export interface DisplayChanges {
  replies: Map<string, number>;
  reactions: Map<string, Map<string, MessageReactionRow | null>>;
}

/** Compute local deltas once per collection update, not once per rendered row. */
export function displayChanges(
  baseMessages: Rows<MessageRow>,
  localMessages: Rows<MessageRow>,
  baseReactions: Rows<MessageReactionRow>,
  localReactions: Rows<MessageReactionRow>,
): DisplayChanges {
  const changes: DisplayChanges = { replies: new Map(), reactions: new Map() };
  const count = (row: MessageRow | undefined, delta: number) => {
    if (row?.parent_id && !row.deleted_at)
      changes.replies.set(row.parent_id, (changes.replies.get(row.parent_id) ?? 0) + delta);
  };
  for (const id of new Set([...baseMessages.keys(), ...localMessages.keys()])) {
    count(baseMessages.get(id), -1);
    count(localMessages.get(id), 1);
  }
  const reaction = (row: MessageReactionRow, value: MessageReactionRow | null) => {
    let updates = changes.reactions.get(row.message_id);
    if (!updates) changes.reactions.set(row.message_id, (updates = new Map()));
    updates.set(row.id, value);
  };
  for (const id of new Set([...baseReactions.keys(), ...localReactions.keys()])) {
    const before = baseReactions.get(id);
    const after = localReactions.get(id);
    if (
      before?.message_id === after?.message_id &&
      before?.emoji === after?.emoji &&
      before?.user_id === after?.user_id
    )
      continue;
    if (before) reaction(before, null);
    if (after) reaction(after, after);
  }
  return changes;
}

/** Incomplete or late Shapes must not replace joined server data. */
export function optimisticDisplay(
  display: MessageDisplayData,
  messageId: string,
  changes: DisplayChanges,
  workspaceEmojis: WorkspaceEmoji[] = [],
): MessageDisplayData {
  const replyDelta = changes.replies.get(messageId) ?? 0;
  const updates = changes.reactions.get(messageId);
  if (!replyDelta && !updates?.size) return display;
  const reactions = new Map(display.reactions.map((row) => [row.id, row]));
  let emojis = display.emojis;
  for (const [id, row] of updates ?? []) {
    if (row) {
      reactions.set(id, row);
      const name = /^:([^:]+):$/.exec(row.emoji)?.[1];
      const emoji =
        name &&
        workspaceEmojis.find((value) => value.name === name || value.aliases?.includes(name));
      if (name && emoji && !emojis[name]) emojis = { ...emojis, [name]: emoji };
    } else reactions.delete(id);
  }
  return {
    ...display,
    reactions: [...reactions.values()],
    emojis,
    reply_count: Math.max(0, display.reply_count + replyDelta),
  };
}
