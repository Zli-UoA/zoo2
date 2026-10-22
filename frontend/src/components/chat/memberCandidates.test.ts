import { describe, expect, it } from "vite-plus/test";

import type { ChannelUserRow, UserRow } from "../../lib/electric/types";
import {
  filterMemberCandidates,
  MEMBER_CANDIDATE_LIMIT,
  sortUsersByLabel,
  userLabel,
} from "./memberCandidates";

function user(id: string, user_name: string, display_name = ""): UserRow {
  return {
    id,
    user_name,
    display_name,
    created_at: "2026-01-02 00:00:00.000",
    updated_at: null,
  };
}

function member(user_id: string): ChannelUserRow {
  return { user_id, channel_id: "channel-1", joined_at: "2026-01-02 00:00:00.000" };
}

describe("userLabel", () => {
  it("prefers the display name, then the user name", () => {
    expect(userLabel(user("u1", "alice", "Alice A"))).toBe("Alice A");
    expect(userLabel(user("u1", "alice"))).toBe("alice");
  });
});

describe("sortUsersByLabel", () => {
  it("sorts by label and leaves the input untouched", () => {
    const input = [user("u1", "carol", "Carol"), user("u2", "alice", "Alice"), user("u3", "bob")];
    const sorted = sortUsersByLabel(input);
    expect(sorted.map((entry) => entry.id)).toEqual(["u2", "u3", "u1"]);
    expect(input.map((entry) => entry.id)).toEqual(["u1", "u2", "u3"]);
  });
});

describe("filterMemberCandidates", () => {
  const users = [
    user("u1", "alice", "Alice"),
    user("u2", "bob", "Bob"),
    user("u3", "carol", "Carol"),
    user("u4", "dave"),
  ];

  it("keeps the given order, limited to the default count", () => {
    expect(filterMemberCandidates({ users, members: [], query: "" })).toEqual([
      users[0],
      users[1],
      users[2],
      users[3],
    ]);
    expect(
      filterMemberCandidates({ users: [...users].reverse(), members: [], query: "a" }).map(
        (entry) => entry.id,
      ),
    ).toEqual(["u4", "u3", "u1"]);
  });

  it("excludes existing members", () => {
    const result = filterMemberCandidates({ users, members: [member("u1")], query: "" });
    expect(result.map((entry) => entry.id)).toEqual(["u2", "u3", "u4"]);
  });

  it("matches display names and user names, ignoring case and padding", () => {
    expect(
      filterMemberCandidates({ users, members: [], query: "  ali " }).map((entry) => entry.id),
    ).toEqual(["u1"]);
    expect(
      filterMemberCandidates({ users, members: [], query: "DAVE" }).map((entry) => entry.id),
    ).toEqual(["u4"]);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterMemberCandidates({ users, members: [], query: "zzz" })).toEqual([]);
  });

  it("respects the limit after filtering", () => {
    expect(
      filterMemberCandidates({ users, members: [], query: "", limit: 2 }).map((entry) => entry.id),
    ).toEqual(["u1", "u2"]);
    expect(MEMBER_CANDIDATE_LIMIT).toBeGreaterThan(0);
  });
});

it("orders Chinese names according to the selected script", () => {
  const input = [user("a", "阿"), user("b", "王"), user("c", "张")];
  expect(sortUsersByLabel(input, "zh-Hans").map(userLabel)).toEqual(["阿", "王", "张"]);
  expect(sortUsersByLabel(input, "zh-Hant").map(userLabel)).toEqual(["王", "张", "阿"]);
});

describe("integration accounts", () => {
  it("excludes retained webhook users from member candidates", () => {
    const bot = { ...user("bot-1", "integration"), is_bot: true };
    expect(
      filterMemberCandidates({ users: [bot, user("human", "alice")], members: [], query: "" }).map(
        (u) => u.id,
      ),
    ).toEqual(["human"]);
  });
});
