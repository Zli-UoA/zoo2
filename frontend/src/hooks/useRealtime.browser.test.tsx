import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { clearFeedCache, saveFeedCache } from "../lib/offline/feedCache";
import { useMessageFeed } from "./useRealtime";

const state = vi.hoisted(() => ({
  isOnline: true,
  namespace: "realtime-cache-test",
  query: {
    data: [] as Array<Record<string, unknown>>,
    isLoading: true,
    isError: false,
    isReady: false,
    isPersistedReady: false,
    hasNextPage: false,
    isFetchingNextPage: false,
    fetchNextPage: vi.fn(),
  },
}));

vi.mock("@tanstack/react-db", async (original) => ({
  ...(await original<typeof import("@tanstack/react-db")>()),
  useLiveInfiniteQuery: () => state.query,
}));

vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({
    messages: () => ({}),
    users: {},
  }),
}));

vi.mock("../lib/offline/offline-context", () => ({
  useOfflineStatus: () => ({
    isOnline: state.isOnline,
    namespace: state.namespace,
  }),
}));

vi.mock("./useOptimisticDisplay", () => ({
  useOptimisticDisplay: (_channelId: string, rows: unknown) => rows,
}));

function makeRow(id: string, content: string) {
  return {
    id,
    channel_id: "c-cache",
    user_id: "u1",
    parent_id: null,
    message_kind: "message",
    content,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    deleted_at: null,
    user_name: "alice",
    display_name: "Alice",
  };
}

describe("useMessageFeed feedCache fallback", () => {
  beforeEach(async () => {
    state.isOnline = true;
    state.query = {
      data: [],
      isLoading: true,
      isError: false,
      isReady: false,
      isPersistedReady: false,
      hasNextPage: false,
      isFetchingNextPage: false,
      fetchNextPage: vi.fn(),
    };
    await clearFeedCache(state.namespace, "c-cache");
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not flash stale cached rows while online loading resolves normally", async () => {
    await saveFeedCache(state.namespace, "c-cache", [makeRow("stale-1", "Old cached row")], 50);

    const { result, rerender } = renderHook(() => useMessageFeed("c-cache"));

    // Wait past IndexedDB read latency (well under the 1500ms stalled-network fallback):
    await new Promise((resolve) => setTimeout(resolve, 120));
    expect(result.current.data).toEqual([]);

    // Live rows arrive from Electric (returned newest-first before reversing):
    state.query = {
      ...state.query,
      data: [makeRow("live-1", "Fresh live message")],
      isLoading: false,
      isReady: true,
    };
    rerender();

    expect(result.current.data?.map((row) => row.id)).toEqual(["live-1"]);
  });

  it("shows cached rows immediately when offline and after the online fallback delay", async () => {
    await saveFeedCache(state.namespace, "c-cache", [makeRow("stale-1", "Cached offline row")], 50);

    // Offline opens cached rows right away.
    state.isOnline = false;
    const offlineHook = renderHook(() => useMessageFeed("c-cache"));
    await waitFor(() => {
      expect(offlineHook.result.current.data?.map((row) => row.id)).toEqual(["stale-1"]);
    });
    offlineHook.unmount();

    // Online stalled loading eventually falls back after the grace period (1500ms).
    state.isOnline = true;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const stalledHook = renderHook(() => useMessageFeed("c-cache"));
    expect(stalledHook.result.current.data).toEqual([]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_600);
    });
    vi.useRealTimers();

    await waitFor(() => {
      expect(stalledHook.result.current.data?.map((row) => row.id)).toEqual(["stale-1"]);
    });
    stalledHook.unmount();
  });
});
