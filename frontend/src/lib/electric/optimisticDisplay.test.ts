import { describe, expect, it } from "vite-plus/test";
import { displayChanges, optimisticDisplay } from "./optimisticDisplay";
import type { MessageDisplayData, MessageReactionRow, MessageRow } from "./types";

const reaction: MessageReactionRow = {
  id: "own",
  message_id: "root",
  user_id: "me",
  emoji: "👍",
  created_at: "2026-01-01",
};
const reply: MessageRow = {
  id: "reply",
  user_id: "me",
  channel_id: "c",
  content: "Reply",
  parent_id: "root",
  created_at: "2026-01-01",
  updated_at: null,
  deleted_at: null,
};
const remote = { ...reaction, id: "peer", user_id: "peer", user_name: "peer" };
const display: MessageDisplayData = {
  emojis: {},
  attachments: [],
  reactions: [remote],
  reply_count: 10,
};
const map = <T extends { id: string }>(...rows: T[]) => new Map(rows.map((row) => [row.id, row]));

describe("joined display optimistic deltas", () => {
  it("does not replace joined data while separate Shapes are empty", () => {
    const delta = displayChanges(
      map<MessageRow>(),
      map<MessageRow>(),
      map<MessageReactionRow>(),
      map<MessageReactionRow>(),
    );
    expect(optimisticDisplay(display, "root", delta)).toBe(display);
  });
  it("immediately adds a local reaction and reply without losing remote data", () => {
    const delta = displayChanges(
      map<MessageRow>(),
      map(reply),
      map<MessageReactionRow>(),
      map(reaction),
    );
    const result = optimisticDisplay(display, "root", delta);
    expect(result.reply_count).toBe(11);
    expect(result.reactions).toEqual([remote, reaction]);
    expect(result.emojis).toBe(display.emojis);
  });
  it("supports optimistic deletions and restores the exact cache after rollback", () => {
    const cached = { ...display, reactions: [remote, reaction] };
    const removed = displayChanges(
      map(reply),
      map<MessageRow>(),
      map(reaction),
      map<MessageReactionRow>(),
    );
    expect(optimisticDisplay(cached, "root", removed).reactions).toEqual([remote]);
    expect(optimisticDisplay(cached, "root", removed).reply_count).toBe(9);
    const rolledBack = displayChanges(map(reply), map(reply), map(reaction), map(reaction));
    expect(optimisticDisplay(cached, "root", rolledBack)).toBe(cached);
  });
  it("counts moved and soft-deleted replies against their previous parents", () => {
    const moved = { ...reply, parent_id: "other" };
    const delta = displayChanges(
      map(reply),
      map(moved),
      map<MessageReactionRow>(),
      map<MessageReactionRow>(),
    );
    expect(optimisticDisplay(display, "root", delta).reply_count).toBe(9);
    expect(optimisticDisplay(display, "other", delta).reply_count).toBe(11);
    const deleted = displayChanges(
      map(reply),
      map({ ...reply, deleted_at: "2026-01-02" }),
      map<MessageReactionRow>(),
      map<MessageReactionRow>(),
    );
    expect(optimisticDisplay(display, "root", deleted).reply_count).toBe(9);
  });
  it("includes picker metadata for a newly added custom reaction and removes it on rollback", () => {
    const own = { ...reaction, emoji: ":alias:" };
    const delta = displayChanges(
      map<MessageRow>(),
      map<MessageRow>(),
      map<MessageReactionRow>(),
      map(own),
    );
    const emoji = {
      name: "wide",
      url: "/wide.gif",
      width: 120,
      height: 20,
      aliases: ["alias"],
      category: "",
    };
    expect(optimisticDisplay(display, "root", delta, [emoji]).emojis.alias).toEqual(emoji);
    expect(
      optimisticDisplay(
        display,
        "root",
        displayChanges(
          map<MessageRow>(),
          map<MessageRow>(),
          map<MessageReactionRow>(),
          map<MessageReactionRow>(),
        ),
        [emoji],
      ),
    ).toBe(display);
  });
});
