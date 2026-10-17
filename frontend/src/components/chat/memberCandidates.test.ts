import { describe, expect, it } from "vite-plus/test";

import type { ChannelUserRow, UserRow } from "../../lib/electric/types";
import { filterMemberCandidates, MEMBER_CANDIDATE_LIMIT, userLabel } from "./memberCandidates";

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

describe("filterMemberCandidates", () => {
  const users = [
    user("u1", "alice", "Alice"),
    user("u2", "bob", "Bob"),
    user("u3", "carol", "Carol"),
    user("u4", "dave"),
  ];

  it("returns users in name order, limited to the default count", () => {
    expect(filterMemberCandidates({ users, members: [], query: "" })).toEqual([
      users[0],
      users[1],
      users[2],
      users[3],
    ]);
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
