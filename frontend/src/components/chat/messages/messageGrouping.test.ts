import { describe, expect, it } from "vite-plus/test";

import type { FeedMessage } from "./messageFormat";
import { buildMessageItems, GROUP_GAP_MS } from "./messageGrouping";

function message(id: string, userId: string, createdAt: string): FeedMessage {
  return {
    id,
    user_id: userId,
    content: id,
    created_at: createdAt,
    user_name: userId,
    display_name: userId,
  };
}

describe("buildMessageItems", () => {
  it("groups consecutive messages from the same author within the gap", () => {
    const items = buildMessageItems([
      message("m1", "alice", "2026-01-02 00:00:00.000"),
      message("m2", "alice", "2026-01-02 00:01:00.000"),
      message("m3", "alice", "2026-01-02 00:02:00.000"),
    ]);

    expect(items.map((item) => item.isGroupStart)).toEqual([true, false, false]);
    expect(items.map((item) => item.isGroupEnd)).toEqual([false, false, true]);
    expect(items.map((item) => item.showDateSeparator)).toEqual([true, false, false]);
  });

  it("breaks the group after the time gap", () => {
    const items = buildMessageItems([
      message("m1", "alice", "2026-01-02 00:00:00.000"),
      message("m2", "alice", "2026-01-02 00:10:00.000"),
    ]);

    expect(items[0].isGroupEnd).toBe(true);
    expect(items[1].isGroupStart).toBe(true);
    expect(GROUP_GAP_MS).toBe(5 * 60 * 1000);
  });

  it("breaks the group when the author changes", () => {
    const items = buildMessageItems([
      message("m1", "alice", "2026-01-02 00:00:00.000"),
      message("m2", "bob", "2026-01-02 00:00:30.000"),
    ]);

    expect(items[0].isGroupEnd).toBe(true);
    expect(items[1].isGroupStart).toBe(true);
  });

  it("adds a date separator when the day changes", () => {
    const items = buildMessageItems([
      message("m1", "alice", "2026-01-02 12:00:00.000"),
      message("m2", "alice", "2026-01-03 12:00:00.000"),
    ]);

    expect(items[0].showDateSeparator).toBe(true);
    expect(items[1].showDateSeparator).toBe(true);
    expect(items[1].isGroupStart).toBe(true);
  });

  it("treats a lone message as a full group", () => {
    const items = buildMessageItems([message("m1", "alice", "2026-01-02 00:00:00.000")]);

    expect(items[0]).toMatchObject({ isGroupStart: true, isGroupEnd: true });
  });

  it("handles an empty list", () => {
    expect(buildMessageItems([])).toEqual([]);
  });
});
