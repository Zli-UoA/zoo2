// @vitest-environment happy-dom
import { renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { useChannelAccess } from "./useRealtime";
import type { ChannelRow } from "../lib/electric/types";

const fixture = vi.hoisted(() => ({
  members: [{ user_id: "other", channel_id: "private" }],
  syncing: true,
}));
vi.mock("@tanstack/react-db", async (original) => ({
  ...(await original<typeof import("@tanstack/react-db")>()),
  useLiveQuery: () => ({
    data: fixture.members,
    isLoading: fixture.syncing,
    isPersistedReady: true,
    isError: false,
  }),
}));
vi.mock("./useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isLoading: false, isAdmin: false }),
}));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ channelUsers: {} }),
}));
const channel = { id: "private", is_private: true } as ChannelRow;
beforeEach(() => {
  fixture.members = [{ user_id: "other", channel_id: "private" }];
  fixture.syncing = true;
});

it("defers a non-membership decision until the current user's row can arrive", () => {
  const { result, rerender } = renderHook(() => useChannelAccess(channel));
  expect(result.current.isLoading).toBe(false);
  expect(result.current.isMembershipPending).toBe(true);
  expect(result.current.canRead).toBe(false);
  fixture.members = [...fixture.members, { user_id: "me", channel_id: "private" }];
  fixture.syncing = false;
  rerender();
  expect(result.current.isMembershipPending).toBe(false);
  expect(result.current.canRead).toBe(true);
});

it("denies access after initial sync confirms the user is not a member", () => {
  fixture.syncing = false;
  const { result } = renderHook(() => useChannelAccess(channel));
  expect(result.current.isMembershipPending).toBe(false);
  expect(result.current.canRead).toBe(false);
});

it("shows a cached membership before initial sync finishes", () => {
  fixture.members = [...fixture.members, { user_id: "me", channel_id: "private" }];
  const { result } = renderHook(() => useChannelAccess(channel));
  expect(result.current.isMembershipPending).toBe(false);
  expect(result.current.canRead).toBe(true);
});
