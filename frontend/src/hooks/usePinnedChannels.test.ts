// @vitest-environment happy-dom
import { act, renderHook, screen, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError } from "../lib/api/client";
import {
  PinnedChannelsProvider,
  channelPinsConflict,
  isChannelPinsLimit,
  isStalePinWrite,
  moveWithin,
  pinsSignature,
  reorderPinsLocally,
  sortPins,
  usePinnedChannels,
} from "./usePinnedChannels";
import type { UserPinnedChannelResponse } from "../lib/api/types";

const fixture = vi.hoisted(() => ({
  api: {
    getChannelPins: vi.fn(),
    pinChannel: vi.fn(),
    unpinChannel: vi.fn(),
    reorderChannelPins: vi.fn(),
  },
  live: undefined as unknown,
  currentUserId: "user-alice",
  collections: {
    config: {},
    userPinnedChannels: () => ({}),
  },
  translate: (key: string) => key,
}));

vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => fixture.collections,
}));

vi.mock("./useCurrentUser", () => ({
  useCurrentUser: () => ({
    currentUserId: fixture.currentUserId,
    isLoading: false,
  }),
}));

vi.mock("../lib/electric/writes", () => ({
  createChannelPinWrites: () => fixture.api,
}));

vi.mock("@tanstack/react-db", () => ({
  useLiveQuery: () => ({ data: fixture.live, isLoading: false, isError: false }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: fixture.translate }),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

vi.mock("../lib/api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/api/client")>();
  return { ...actual, createConfiguredApiClient: () => ({}) };
});

function pin(channelId: string, position: number): UserPinnedChannelResponse {
  return {
    user_id: "user-alice",
    channel_id: channelId,
    position,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: "2026-01-01 00:00:00.000",
  };
}

function conflict(version: number, pins: UserPinnedChannelResponse[]): ApiError {
  return new ApiError(409, "conflict", {
    error: "conflict",
    code: "CHANNEL_PINS_CONFLICT",
    version,
    pins,
  });
}

function wrapper({ children }: { children: ReactNode }) {
  return createElement(PinnedChannelsProvider, null, children);
}

beforeEach(() => {
  fixture.live = undefined;
  fixture.currentUserId = "user-alice";
  fixture.translate = (key: string) => key;
  fixture.api.getChannelPins.mockReset().mockResolvedValue({ pins: [], version: 0 });
  fixture.api.pinChannel.mockReset();
  fixture.api.unpinChannel.mockReset();
  fixture.api.reorderChannelPins.mockReset();
});

describe("pure pin helpers", () => {
  it("sorts by position and channel id", () => {
    const pins = [pin("b", 1), pin("c", 1), pin("a", 0)];
    expect(sortPins(pins).map((item) => item.channel_id)).toEqual(["a", "b", "c"]);
    expect(pinsSignature(pins)).toBe("a:0|b:1|c:1");
  });

  it("moves an item without losing the others", () => {
    expect(moveWithin(["a", "b", "c"], "a", 1)).toEqual(["b", "a", "c"]);
    expect(moveWithin(["a", "b", "c"], "c", 0)).toEqual(["c", "a", "b"]);
    expect(moveWithin(["a", "b", "c"], "a", 99)).toEqual(["b", "c", "a"]);
  });

  it("treats a queued write as stale when the generation or the user changes", () => {
    expect(isStalePinWrite(1, 1, "alice", "alice")).toBe(false);
    // ユーザー切り替えの effect が走る前でも、別ユーザーなら送らない。
    expect(isStalePinWrite(1, 1, "alice", "bob")).toBe(true);
    expect(isStalePinWrite(1, 2, "alice", "alice")).toBe(true);
  });

  it("applies a reorder optimistically while keeping archived slots", () => {
    const pins = [pin("a", 0), { ...pin("old", 1), archived: true }, pin("c", 2)];
    expect(reorderPinsLocally(pins, ["c", "a"]).map((item) => item.channel_id)).toEqual([
      "c",
      "old",
      "a",
    ]);
    expect(reorderPinsLocally(pins, ["c", "a"]).map((item) => item.position)).toEqual([0, 1, 2]);
  });

  it("keeps a pin out of the visible order even when its archived flag is stale", () => {
    // チャンネルが後からアーカイブされても pin 行の archived は古いまま。
    // nextOrder に含まれないピンを有効扱いすると、b が重複する。
    const pins = [pin("a", 0), pin("old", 1), pin("b", 2)];
    expect(reorderPinsLocally(pins, ["b", "a"]).map((item) => item.channel_id)).toEqual([
      "b",
      "old",
      "a",
    ]);
  });

  it("moves a pin whose stale archived flag is true when it is visible again", () => {
    const pins = [pin("a", 0), { ...pin("old", 1), archived: true }, pin("c", 2)];
    expect(reorderPinsLocally(pins, ["c", "old", "a"]).map((item) => item.channel_id)).toEqual([
      "c",
      "old",
      "a",
    ]);
  });

  it("recognizes only the documented conflict and limit bodies", () => {
    const cause = conflict(3, [pin("a", 0)]);
    expect(channelPinsConflict(cause)).toEqual({ version: 3, pins: [pin("a", 0)] });
    expect(channelPinsConflict(new ApiError(409, "other", { code: "OTHER" }))).toBeNull();
    const limit = new ApiError(400, "limit", { code: "CHANNEL_PINS_LIMIT_EXCEEDED" });
    expect(isChannelPinsLimit(limit)).toBe(true);
    expect(isChannelPinsLimit(cause)).toBe(false);
  });
});

describe("usePinnedChannels", () => {
  it("loads the authoritative version and tracks the version returned by writes", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [pin("a", 0)], version: 2 });
    fixture.api.pinChannel.mockResolvedValue({
      data: { pin: pin("b", 1), version: 3, pins: [pin("a", 0), pin("b", 1)] },
      txid: "9",
    });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));
    expect(result.current.isPinned("a")).toBe(true);

    await act(async () => {
      await result.current.pinChannel("b");
    });

    expect(fixture.api.pinChannel).toHaveBeenCalledWith("b", 2);
    expect(result.current.version).toBe(3);
    expect(result.current.isPinned("b")).toBe(true);
    expect(result.current.announcement).toBe("channelPins.announce.pinned");
  });

  it("retries a pin on top of the latest pins after a 409 conflict", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [], version: 0 });
    fixture.api.pinChannel
      .mockRejectedValueOnce(conflict(5, [pin("other", 0)]))
      .mockResolvedValueOnce({
        data: {
          pin: pin("b", 1),
          version: 6,
          pins: [pin("other", 0), pin("b", 1)],
        },
        txid: "10",
      });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.pinChannel("b");
    });

    expect(fixture.api.pinChannel).toHaveBeenNthCalledWith(1, "b", 0);
    expect(fixture.api.pinChannel).toHaveBeenNthCalledWith(2, "b", 5);
    expect(result.current.version).toBe(6);
    expect(result.current.isPinned("b")).toBe(true);
    expect(result.current.error).toBeNull();
  });

  it("adopts the latest state when the channel was already pinned elsewhere", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [], version: 0 });
    fixture.api.pinChannel.mockRejectedValueOnce(conflict(4, [pin("b", 0)]));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.pinChannel("b");
    });

    expect(fixture.api.pinChannel).toHaveBeenCalledTimes(1);
    expect(result.current.version).toBe(4);
    expect(result.current.isPinned("b")).toBe(true);
    expect(result.current.announcement).toBe("channelPins.announce.pinned");
  });

  it("adopts the latest state when the channel was already unpinned elsewhere", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [pin("a", 0)], version: 3 });
    fixture.api.unpinChannel.mockRejectedValueOnce(conflict(4, []));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(3));

    await act(async () => {
      await result.current.unpinChannel("a");
    });

    expect(fixture.api.unpinChannel).toHaveBeenCalledTimes(1);
    expect(result.current.version).toBe(4);
    expect(result.current.pins).toEqual([]);
    expect(result.current.announcement).toBe("channelPins.announce.unpinned");
  });

  it("does not resurrect a pin that another device removed during a reorder", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 2,
    });
    fixture.api.reorderChannelPins.mockRejectedValueOnce(conflict(3, [pin("b", 0)]));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    await act(async () => {
      await result.current.reorderPins(["b", "a"], "a");
    });

    expect(fixture.api.reorderChannelPins).toHaveBeenCalledTimes(1);
    expect(result.current.version).toBe(3);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["b"]);
    expect(result.current.announcement).toBe("channelPins.announce.reorderConflict");
    expect(result.current.error).toBeNull();
  });

  it("retries a reorder on top of the latest order after a 409 conflict", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1), pin("c", 2)],
      version: 2,
    });
    fixture.api.reorderChannelPins
      .mockRejectedValueOnce(conflict(3, [pin("a", 0), pin("c", 1), pin("b", 2)]))
      .mockResolvedValueOnce({
        data: { version: 4, pins: [pin("c", 0), pin("a", 1), pin("b", 2)] },
        txid: "11",
      });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    await act(async () => {
      await result.current.reorderPins(["b", "a", "c"], "a");
    });

    expect(fixture.api.reorderChannelPins).toHaveBeenNthCalledWith(1, ["b", "a", "c"], 2);
    // 最新の並びで `a` の直後に置いた `c` の手前へ、`a` だけを入れ直す。
    expect(fixture.api.reorderChannelPins).toHaveBeenNthCalledWith(2, ["a", "c", "b"], 3);
    expect(result.current.version).toBe(4);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["c", "a", "b"]);
  });

  it("keeps pins added on another device when retrying a reorder", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 2,
    });
    fixture.api.reorderChannelPins
      .mockRejectedValueOnce(conflict(3, [pin("a", 0), pin("b", 1), pin("new", 2)]))
      .mockResolvedValueOnce({
        data: {
          version: 4,
          pins: [pin("b", 0), pin("a", 1), pin("new", 2)],
        },
        txid: "12",
      });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    await act(async () => {
      await result.current.reorderPins(["b", "a"], "a");
    });

    expect(fixture.api.reorderChannelPins).toHaveBeenNthCalledWith(2, ["b", "a", "new"], 3);
    expect(result.current.version).toBe(4);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["b", "a", "new"]);
  });

  it("does not resurrect a pin archived on another device during a reorder", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 2,
    });
    fixture.api.reorderChannelPins.mockRejectedValueOnce(
      conflict(3, [{ ...pin("a", 0), archived: true }, pin("b", 1)]),
    );

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    await act(async () => {
      await result.current.reorderPins(["b", "a"], "a");
    });

    expect(fixture.api.reorderChannelPins).toHaveBeenCalledTimes(1);
    expect(result.current.version).toBe(3);
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);
    expect(result.current.announcement).toBe("channelPins.announce.reorderArchived");
    expect(result.current.error).toBeNull();
  });

  it("refreshes the version when Electric reports a change from another device", async () => {
    fixture.api.getChannelPins
      .mockResolvedValueOnce({ pins: [pin("a", 0)], version: 1 })
      .mockResolvedValueOnce({ pins: [pin("a", 0), pin("b", 1)], version: 2 });
    fixture.live = [pin("a", 0)];

    const { result, rerender } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(1));

    fixture.live = [pin("a", 0), pin("b", 1)];
    rerender();

    await waitFor(() => expect(result.current.version).toBe(2), { timeout: 2_000 });
    expect(result.current.isPinned("b")).toBe(true);
  });

  it("ignores a slow initial GET that resolves after a write", async () => {
    let resolveInitial: ((value: unknown) => void) | undefined;
    fixture.api.getChannelPins.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInitial = resolve;
        }),
    );
    fixture.api.pinChannel.mockResolvedValue({
      data: { pin: pin("b", 1), version: 3, pins: [pin("a", 0), pin("b", 1)] },
      txid: "13",
    });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.pinChannel("b");
    });
    expect(result.current.version).toBe(3);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a", "b"]);

    // 初回 GET が書き込みより後に古い版で返っても、採用して巻き戻さない。
    await act(async () => {
      resolveInitial?.({ pins: [], version: 0 });
      await Promise.resolve();
    });

    expect(result.current.version).toBe(3);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a", "b"]);
  });

  it("does not let a delayed write response roll back a newer version", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [pin("a", 0)], version: 3 });
    // 遅れて届いた書き込みの応答は、すでに採用した版より古い。
    fixture.api.pinChannel.mockResolvedValue({
      data: { pin: pin("b", 1), version: 2, pins: [] },
      txid: "16",
    });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(3));

    await act(async () => {
      await result.current.pinChannel("b");
    });

    expect(fixture.api.pinChannel).toHaveBeenCalledWith("b", 3);
    expect(result.current.version).toBe(3);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a"]);
  });

  it("does not send a queued write after the account changes", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [], version: 0 });
    let resolveFirst: ((value: unknown) => void) | undefined;
    fixture.api.pinChannel.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
    );

    const { result, rerender } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => {
      void result.current.pinChannel("a");
    });
    await waitFor(() => expect(fixture.api.pinChannel).toHaveBeenCalledTimes(1));
    // 1 件目の応答を待つ間に、2 件目をキューへ入れる。
    act(() => {
      void result.current.pinChannel("b");
    });

    // 1 件目が終わる前にアカウントが切り替わる。
    fixture.currentUserId = "user-bob";
    rerender();

    await act(async () => {
      resolveFirst?.({
        data: { pin: pin("a", 0), version: 1, pins: [pin("a", 0)] },
        txid: "1",
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    // 2 件目は新しいユーザーとして送らず、古い応答も反映しない。
    expect(fixture.api.pinChannel).toHaveBeenCalledTimes(1);
    expect(fixture.api.pinChannel).toHaveBeenCalledWith("a", 0);
  });

  it("keeps pins and pending writes when only the i18n language changes", async () => {
    let resolvePin: ((value: unknown) => void) | undefined;
    fixture.api.getChannelPins.mockResolvedValue({ pins: [pin("a", 0)], version: 2 });
    fixture.api.pinChannel.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePin = resolve;
        }),
    );

    const { result, rerender } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));
    expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1);

    act(() => {
      void result.current.pinChannel("b");
    });
    await waitFor(() => expect(fixture.api.pinChannel).toHaveBeenCalledTimes(1));
    expect(result.current.pending.has("b")).toBe(true);

    // 言語を変えると `t` の関数が変わる。状態と保留中の書き込みを捨てない。
    fixture.translate = (key: string) => `ja:${key}`;
    rerender();

    expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1);
    expect(result.current.version).toBe(2);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a"]);
    expect(result.current.pending.has("b")).toBe(true);

    await act(async () => {
      resolvePin?.({
        data: { pin: pin("b", 1), version: 3, pins: [pin("a", 0), pin("b", 1)] },
        txid: "17",
      });
    });

    await waitFor(() => expect(result.current.version).toBe(3));
    expect(result.current.isPinned("b")).toBe(true);
    expect(result.current.pending.has("b")).toBe(false);
  });

  it("refetches for another device's Electric update right after a write", async () => {
    fixture.api.getChannelPins
      .mockResolvedValueOnce({ pins: [pin("a", 0)], version: 1 })
      .mockResolvedValueOnce({ pins: [pin("a", 0), pin("b", 1)], version: 3 });
    fixture.api.pinChannel.mockResolvedValue({
      data: { pin: pin("c", 1), version: 2, pins: [pin("a", 0), pin("c", 1)] },
      txid: "14",
    });
    fixture.live = [pin("a", 0)];

    const { result, rerender } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(1));

    await act(async () => {
      await result.current.pinChannel("c");
    });
    expect(result.current.version).toBe(2);

    // 書き込み直後の 1 秒以内でも、別端末の変更なら取り直す。
    fixture.live = [pin("a", 0), pin("b", 1)];
    rerender();

    await waitFor(() => expect(result.current.version).toBe(3), { timeout: 2_000 });
    expect(result.current.isPinned("b")).toBe(true);
  });

  it("clears the load error when a later fetch succeeds", async () => {
    fixture.api.getChannelPins
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({ pins: [pin("a", 0)], version: 1 });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.error).not.toBeNull());

    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.error).toBeNull();
    expect(result.current.version).toBe(1);
    expect(result.current.isPinned("a")).toBe(true);
  });

  it("ignores an older GET success that arrives after a newer same-version success", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    fixture.api.getChannelPins
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce({
        pins: [{ ...pin("a", 0), archived: true }, pin("b", 1)],
        version: 2,
      });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);

    // 先発の成功が後から届いても、後発が採用した archived を巻き戻さない。
    await act(async () => {
      resolveFirst?.({ pins: [pin("a", 0), pin("b", 1)], version: 2 });
      await Promise.resolve();
    });

    expect(result.current.version).toBe(2);
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);
  });

  it("accepts an older-started GET when it returns a newer version", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    fixture.api.getChannelPins
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce({
        pins: [{ ...pin("a", 0), archived: true }, pin("b", 1)],
        version: 5,
      });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.version).toBe(5);

    // 先発でもより新しい版なら採用し、後発の古い版で表示を固定しない。
    await act(async () => {
      resolveFirst?.({ pins: [pin("a", 0), pin("b", 1)], version: 6 });
      await Promise.resolve();
    });

    expect(result.current.version).toBe(6);
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBeUndefined();
  });

  it("ignores a superseded GET failure that arrives after a newer success", async () => {
    let rejectFirst: ((reason?: unknown) => void) | undefined;
    fixture.api.getChannelPins
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectFirst = reject;
          }),
      )
      .mockResolvedValueOnce({ pins: [pin("a", 0)], version: 1 });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(fixture.api.getChannelPins).toHaveBeenCalledTimes(1));

    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.version).toBe(1);
    expect(result.current.error).toBeNull();

    // 先発の GET が後から失敗しても、後発の成功をエラー表示で上書きしない。
    await act(async () => {
      rejectFirst?.(new Error("offline"));
      await Promise.resolve();
    });

    expect(result.current.error).toBeNull();
    expect(result.current.version).toBe(1);
  });

  it("does not roll back a reorder after adopting a conflict's authoritative order", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 1,
    });
    fixture.api.reorderChannelPins
      .mockRejectedValueOnce(conflict(2, [pin("b", 0), pin("a", 1)]))
      .mockRejectedValueOnce(new Error("offline"));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(1));

    await act(async () => {
      await result.current.reorderPins(["b", "a"], "b");
    });

    // 409 で版 2 の並びを採用済みなので、再試行の通信エラーで版 1 へ戻さない。
    expect(result.current.version).toBe(2);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["b", "a"]);
    expect(result.current.error).not.toBeNull();
  });

  it("does not roll back an archived flag adopted from a same-version conflict", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 2,
    });
    // 並びは同じで archived だけが変わる 409。版も署名も変わらないため、
    // `adoptCount` の採用回数が増えていなければ再試行失敗時に archived を失う。
    fixture.api.reorderChannelPins
      .mockRejectedValueOnce(conflict(2, [{ ...pin("a", 0), archived: true }, pin("b", 1)]))
      .mockRejectedValueOnce(new Error("offline"));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    await act(async () => {
      await result.current.reorderPins(["a", "b"], "b");
    });

    // 同じ版でも 409 の archived を採用済みなら、再試行の通信エラーで巻き戻さない。
    expect(result.current.version).toBe(2);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a", "b"]);
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);
    expect(result.current.error).not.toBeNull();
  });

  it("does not roll back when a concurrent GET adopts a same-version archived change", async () => {
    fixture.api.getChannelPins
      .mockResolvedValueOnce({ pins: [pin("a", 0), pin("b", 1)], version: 2 })
      .mockResolvedValueOnce({
        pins: [{ ...pin("a", 0), archived: true }, pin("b", 1)],
        version: 2,
      });
    let rejectReorder: ((reason?: unknown) => void) | undefined;
    fixture.api.reorderChannelPins.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectReorder = reject;
        }),
    );

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(2));

    let reorderPromise: Promise<void> | undefined;
    await act(async () => {
      reorderPromise = result.current.reorderPins(["a", "b"], "b");
      await Promise.resolve();
    });
    await waitFor(() => expect(fixture.api.reorderChannelPins).toHaveBeenCalledTimes(1));

    // 並び替え中に GET が同じ版で archived だけを変えて採用する。
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);

    await act(async () => {
      rejectReorder?.(new Error("offline"));
      await reorderPromise;
    });

    // GET で採用した archived を、並び替えの失敗で巻き戻さない。
    expect(result.current.version).toBe(2);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a", "b"]);
    expect(result.current.pins.find((item) => item.channel_id === "a")?.archived).toBe(true);
    expect(result.current.error).not.toBeNull();
  });

  it("rolls back when a conflict from an older version is ignored", async () => {
    fixture.api.getChannelPins.mockResolvedValue({
      pins: [pin("a", 0), pin("b", 1)],
      version: 3,
    });
    fixture.api.reorderChannelPins
      .mockRejectedValueOnce(conflict(2, [pin("b", 0), pin("a", 1)]))
      .mockRejectedValueOnce(new Error("offline"));

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.version).toBe(3));

    await act(async () => {
      await result.current.reorderPins(["b", "a"], "b");
    });

    // 古い版の 409 は採用しないので、通信エラー時は楽観更新前へ戻す。
    expect(result.current.version).toBe(3);
    expect(result.current.pins.map((item) => item.channel_id)).toEqual(["a", "b"]);
    expect(result.current.error).not.toBeNull();
  });

  it("announces every operation, including repeats, in an always mounted live region", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [], version: 0 });
    fixture.api.pinChannel.mockResolvedValue({
      data: { pin: pin("b", 0), version: 1, pins: [pin("b", 0)] },
      txid: "15",
    });

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));
    // 同じテストファイルの以前の描画が残っている環境でも、この Provider の領域を見る。
    const status = screen.getAllByRole("status").at(-1)!;
    expect(status.textContent).toBe("");

    await act(async () => {
      await result.current.pinChannel("b");
    });
    const firstAnnouncement = status.querySelector("span");
    expect(status.textContent).toBe("channelPins.announce.pinned");

    // 同じ操作を続けても、読み上げ領域の要素を差し替えて必ず読み上げ直す。
    await act(async () => {
      await result.current.pinChannel("b");
    });
    const secondAnnouncement = status.querySelector("span");
    expect(secondAnnouncement).not.toBe(firstAnnouncement);
    expect(status.textContent).toBe("channelPins.announce.pinned");
  });

  it("shows the limit message instead of a generic conflict", async () => {
    fixture.api.getChannelPins.mockResolvedValue({ pins: [], version: 0 });
    fixture.api.pinChannel.mockRejectedValueOnce(
      new ApiError(400, "limit", { error: "limit", code: "CHANNEL_PINS_LIMIT_EXCEEDED" }),
    );

    const { result } = renderHook(() => usePinnedChannels(), { wrapper });
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.pinChannel("b");
    });

    expect(result.current.error).toBe("channelPins.errors.limitExceeded");
    expect(result.current.isPinned("b")).toBe(false);
  });
});
