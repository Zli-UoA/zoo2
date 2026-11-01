import { describe, expect, it } from "vite-plus/test";

import { groupReactions, reactionUserLabel } from "./reactions";

const row = {
  id: "r1",
  message_id: "m1",
  user_id: "u1",
  emoji: "kitchen:1f600_2764",
  created_at: "2026-01-01T00:00:00Z",
};

describe("reaction groups", () => {
  it("distinguishes users with the same display name by username", () => {
    expect(reactionUserLabel({ display_name: "Alex", user_name: "alex1" })).toBe("Alex (@alex1)");
    expect(reactionUserLabel({ display_name: "Alex", user_name: "alex2" })).toBe("Alex (@alex2)");
  });
  it("counts people, combines equivalent Kitchen spellings and identifies only my reaction", () => {
    const result = groupReactions(
      [
        row,
        { ...row, id: "r2", user_id: "u2", emoji: "kitchen:2764-fe0f_1f600" },
        { ...row, id: "r3", user_id: "u2" },
        { ...row, id: "r4", message_id: "m2", user_id: "u2" },
      ],
      "u1",
    );
    expect(result.get("m1")).toEqual([
      { emoji: row.emoji, count: 2, ownId: "r1", userIds: ["u1", "u2"] },
    ]);
    expect(result.get("m2")).toEqual([
      { emoji: row.emoji, count: 1, ownId: null, userIds: ["u2"] },
    ]);
    expect(groupReactions([row], null).get("m1")?.[0].ownId).toBeNull();
  });
});
