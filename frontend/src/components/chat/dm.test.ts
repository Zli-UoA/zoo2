import { describe, expect, it } from "vite-plus/test";

import type { UserRow } from "../../lib/electric/types";
import {
  DM_MAX_MEMBERS,
  DM_MAX_TARGETS,
  filterDmCandidates,
  isGroupDm,
  parseOneToOneDmIds,
  resolveDmDisplayName,
  waitForDmAvailability,
} from "./dm";

type DmUser = Pick<UserRow, "id" | "display_name" | "user_name" | "is_bot">;

const users: DmUser[] = [
  { id: "u1", user_name: "alice", display_name: "Alice" },
  { id: "u2", user_name: "bob", display_name: "Bob" },
  { id: "u3", user_name: "carol", display_name: "Carol" },
  { id: "u4", user_name: "dave", display_name: "Dave" },
];

function members(...ids: string[]) {
  return ids.map((user_id) => ({ user_id }));
}

describe("DM channel names", () => {
  it("recognizes group and 1:1 channel names", () => {
    expect(isGroupDm("dm:group:abc")).toBe(true);
    expect(isGroupDm("dm:u1:u2")).toBe(false);
    expect(parseOneToOneDmIds("dm:u1:u2")).toEqual(["u1", "u2"]);
    expect(parseOneToOneDmIds("dm:group:abc")).toBeNull();
    expect(parseOneToOneDmIds("general")).toBeNull();
    expect(parseOneToOneDmIds("dm:only-one")).toBeNull();
  });

  it("uses the other participant for a 1:1 conversation", () => {
    expect(
      resolveDmDisplayName({
        channelName: "dm:u1:u2",
        members: members("u1", "u2"),
        users,
        currentUserId: "u1",
        locale: "en",
      }),
    ).toEqual({ names: ["Bob"], remaining: 0 });
  });

  it("lists other participants for a group and summarizes the rest", () => {
    expect(
      resolveDmDisplayName({
        channelName: "dm:group:abc",
        members: members("u1", "u2", "u3", "u4"),
        users,
        currentUserId: "u1",
        locale: "en",
        maxNames: 2,
      }),
    ).toEqual({ names: ["Bob", "Carol"], remaining: 1 });
  });

  it("sorts names using the selected locale", () => {
    expect(
      resolveDmDisplayName({
        channelName: "dm:group:abc",
        members: members("u1", "u2", "u3"),
        users,
        currentUserId: "u4",
        locale: "en",
      }).names,
    ).toEqual(["Alice", "Bob", "Carol"]);
  });

  it("ignores participants whose user row is not loaded yet", () => {
    expect(
      resolveDmDisplayName({
        channelName: "dm:u1:missing",
        members: members("u1"),
        users,
        currentUserId: "u1",
        locale: "en",
      }),
    ).toEqual({ names: [], remaining: 0 });
  });

  it("falls back to the channel participants when the current user is unknown", () => {
    expect(
      resolveDmDisplayName({
        channelName: "dm:group:abc",
        members: members("u1", "u2"),
        users,
        currentUserId: null,
        locale: "en",
      }).names,
    ).toEqual(["Alice", "Bob"]);
  });
});

describe("DM candidate filtering", () => {
  it("excludes the current user and already selected users", () => {
    expect(
      filterDmCandidates({
        users,
        currentUserId: "u1",
        selectedIds: ["u2"],
        query: "",
      }).map((user) => user.id),
    ).toEqual(["u3", "u4"]);
  });

  it("matches display names and user names, ignoring case and padding", () => {
    expect(
      filterDmCandidates({
        users,
        currentUserId: "u1",
        selectedIds: [],
        query: "  CAR  ",
      }).map((user) => user.id),
    ).toEqual(["u3"]);
    expect(
      filterDmCandidates({
        users,
        currentUserId: "u1",
        selectedIds: [],
        query: "dave",
      }).map((user) => user.id),
    ).toEqual(["u4"]);
  });

  it("excludes bot users from the candidates", () => {
    const withBot: DmUser[] = [
      ...users,
      { id: "bot-1", user_name: "zoo-bot", display_name: "Zoo Bot", is_bot: true },
    ];
    expect(
      filterDmCandidates({
        users: withBot,
        currentUserId: "u1",
        selectedIds: [],
        query: "zoo",
      }),
    ).toEqual([]);
  });

  it("keeps the maximum group size at ten including yourself", () => {
    expect(DM_MAX_MEMBERS).toBe(10);
    expect(DM_MAX_TARGETS).toBe(9);
  });
});

describe("waitForDmAvailability", () => {
  it("returns true when the channel and current membership are already available", async () => {
    let slept = false;
    const source = {
      channels: { has: (channelId: string) => channelId === "channel-1" },
      channelUsers: { has: (key: string) => key === "user-1:channel-1" },
    };
    await expect(
      waitForDmAvailability(source, "channel-1", "user-1", {
        sleep: async () => {
          slept = true;
        },
      }),
    ).resolves.toBe(true);
    expect(slept).toBe(false);
  });

  it("waits for both shapes to reflect the new DM", async () => {
    let now = 0;
    const memberships = new Set<string>();
    const source = {
      channels: { has: (channelId: string) => channelId === "channel-1" },
      channelUsers: { has: (key: string) => memberships.has(key) },
    };
    const result = await waitForDmAvailability(source, "channel-1", "user-1", {
      timeoutMs: 1_000,
      pollMs: 100,
      now: () => now,
      sleep: async (ms) => {
        now += ms;
        memberships.add("user-1:channel-1");
      },
    });
    expect(result).toBe(true);
    expect(now).toBe(100);
  });

  it("returns false when the shapes do not reflect the DM before the timeout", async () => {
    let now = 0;
    const source = {
      channels: { has: () => false },
      channelUsers: { has: () => false },
    };
    const result = await waitForDmAvailability(source, "channel-1", "user-1", {
      timeoutMs: 250,
      pollMs: 100,
      now: () => now,
      sleep: async (ms) => {
        now += ms;
      },
    });
    expect(result).toBe(false);
    expect(now).toBe(300);
  });
});
