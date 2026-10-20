import { describe, expect, it } from "vite-plus/test";

import { canAddChannelMember, canKickChannelMember, canManageChannel } from "./channelPermissions";

describe("canManageChannel", () => {
  it("allows the creator or a workspace admin", () => {
    expect(canManageChannel({ isAdmin: false, isCreator: true })).toBe(true);
    expect(canManageChannel({ isAdmin: true, isCreator: false })).toBe(true);
    expect(canManageChannel({ isAdmin: true, isCreator: true })).toBe(true);
    expect(canManageChannel({ isAdmin: false, isCreator: false })).toBe(false);
  });
});

describe("canAddChannelMember", () => {
  it("matches the backend conditions", () => {
    const base = { isAdmin: false, isCreator: false, isPrivate: false, isMember: false };
    expect(canAddChannelMember({ ...base, isAdmin: true })).toBe(true);
    expect(canAddChannelMember({ ...base, isCreator: true })).toBe(true);
    expect(canAddChannelMember({ ...base, isMember: true })).toBe(true);
    expect(canAddChannelMember({ ...base, isPrivate: true, isMember: true })).toBe(false);
    expect(canAddChannelMember(base)).toBe(false);
  });
});

describe("canKickChannelMember", () => {
  const base = {
    actorId: "alice",
    actorCanManage: true,
    channelCreatorId: "alice",
    targetUserId: "bob",
  };

  it("allows the creator or an admin to kick a regular member", () => {
    expect(canKickChannelMember(base)).toBe(true);
    expect(canKickChannelMember({ ...base, actorId: "admin", channelCreatorId: "alice" })).toBe(
      true,
    );
  });

  it("rejects non-managers, self, and the channel creator", () => {
    expect(canKickChannelMember({ ...base, actorCanManage: false })).toBe(false);
    expect(canKickChannelMember({ ...base, actorId: null })).toBe(false);
    expect(canKickChannelMember({ ...base, targetUserId: "alice" })).toBe(false);
    expect(canKickChannelMember({ ...base, actorId: "admin", targetUserId: "admin" })).toBe(false);
  });
});
