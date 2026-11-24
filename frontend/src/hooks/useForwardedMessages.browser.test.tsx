import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import {
  FORWARD_OK_TTL_MS,
  forgetForwardIds,
  peekForwardExpiresAt,
  resetForwardCacheForTests,
} from "../lib/api/forwards";
import type { ForwardResolutionItem } from "../lib/api/types";
import { useForwardedMessages } from "./useForwardedMessages";

const state = vi.hoisted(() => ({
  apiBaseUrl: "https://a.example",
  currentUserId: "alice" as string | null,
  post: vi.fn(),
}));

vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({
    config: { apiBaseUrl: state.apiBaseUrl },
  }),
}));

vi.mock("./useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: state.currentUserId }),
}));

vi.mock("../lib/api/client", () => ({
  createConfiguredApiClient: () => ({
    post: state.post,
  }),
}));

function okForward(id: string, content: string): ForwardResolutionItem {
  return {
    status: "ok",
    id,
    channel_id: "ch-priv",
    channel_name: "secret",
    is_private: true,
    is_dm: false,
    parent_id: null,
    user_id: "alice",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    content,
    card: null,
    components: null,
    message_kind: "message",
    author: {
      id: "alice",
      user_name: "alice",
      display_name: "Alice",
    },
    emojis: {},
    attachments: [],
    has_forwards: false,
  };
}

describe("useForwardedMessages", () => {
  beforeEach(() => {
    state.apiBaseUrl = "https://a.example";
    state.currentUserId = "alice";
    state.post.mockReset();
    resetForwardCacheForTests();
  });

  afterEach(() => {
    resetForwardCacheForTests();
  });

  it("drops local resolved entries immediately when scope changes without leaking to the next user", async () => {
    state.post.mockImplementation(async () => ({
      data: {
        items:
          state.currentUserId === "alice"
            ? { "m-priv": okForward("m-priv", "alice secret") }
            : { "m-priv": { status: "unavailable" } },
      },
      txid: null,
    }));

    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => useForwardedMessages(ids),
      {
        initialProps: { ids: ["m-priv"] },
      },
    );

    await waitFor(() => {
      expect(result.current.entries.get("m-priv")).toMatchObject({
        status: "ok",
        content: "alice secret",
      });
    });

    // 別ユーザーへ切り替わった直後の描画から、前ユーザーの局所状態（alice secret）を絶対に返さない
    state.currentUserId = "bob";
    rerender({ ids: ["m-priv"] });

    expect(result.current.entries.get("m-priv")).toEqual({ status: "loading" });

    await waitFor(() => {
      expect(result.current.entries.get("m-priv")).toEqual({ status: "unavailable" });
    });

    // apiBaseUrl の切り替えでも同様に分離され、前の結果を見せない
    state.apiBaseUrl = "https://b.example";
    state.post.mockImplementation(async () => ({
      data: { items: { "m-priv": okForward("m-priv", "server b content") } },
      txid: null,
    }));
    rerender({ ids: ["m-priv"] });

    expect(result.current.entries.get("m-priv")).toEqual({ status: "loading" });

    await waitFor(() => {
      expect(result.current.entries.get("m-priv")).toMatchObject({
        status: "ok",
        content: "server b content",
      });
    });
  });

  it("re-resolves when forgetForwardIds invalidates an entry in the active scope", async () => {
    let edited = false;
    state.post.mockImplementation(async () => ({
      data: {
        items: { "m-1": okForward("m-1", edited ? "after edit" : "before edit") },
      },
      txid: null,
    }));

    const { result } = renderHook(() => useForwardedMessages(["m-1"]));
    await waitFor(() => {
      expect(result.current.entries.get("m-1")).toMatchObject({
        status: "ok",
        content: "before edit",
      });
    });

    edited = true;
    act(() => {
      forgetForwardIds({ apiBaseUrl: "https://a.example", currentUserId: "alice" }, ["m-1"]);
    });

    await waitFor(() => {
      expect(result.current.entries.get("m-1")).toMatchObject({
        status: "ok",
        content: "after edit",
      });
    });
  });

  it("updates a resolved entry to deleted after invalidation or tab return past TTL", async () => {
    let deleted = false;
    state.post.mockImplementation(async () => ({
      data: {
        items: {
          "m-del": deleted ? { status: "deleted" } : okForward("m-del", "original body"),
        },
      },
      txid: null,
    }));

    const { result } = renderHook(() => useForwardedMessages(["m-del"]));
    await waitFor(() => {
      expect(result.current.entries.get("m-del")).toMatchObject({
        status: "ok",
        content: "original body",
      });
    });

    deleted = true;
    act(() => {
      forgetForwardIds({ apiBaseUrl: "https://a.example", currentUserId: "alice" }, ["m-del"]);
    });
    await waitFor(() => {
      expect(result.current.entries.get("m-del")).toEqual({ status: "deleted" });
    });

    // 次に未削除の投稿を解決し、TTL 経過後のタブ復帰（visibilitychange）でも再解決されることを検証する
    resetForwardCacheForTests();
    let secondDeleted = false;
    state.post.mockImplementation(async () => ({
      data: {
        items: {
          "m-ttl": secondDeleted ? { status: "deleted" } : okForward("m-ttl", "still live"),
        },
      },
      txid: null,
    }));

    const visibilityDesc = Object.getOwnPropertyDescriptor(document, "visibilityState");
    let now = 1_000_000;
    const nowSpy = vi.spyOn(Date, "now").mockImplementation(() => now);
    try {
      const ttlHook = renderHook(() => useForwardedMessages(["m-ttl"]));
      await waitFor(() => {
        expect(ttlHook.result.current.entries.get("m-ttl")).toMatchObject({
          status: "ok",
          content: "still live",
        });
      });

      secondDeleted = true;
      now += FORWARD_OK_TTL_MS + 1;
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        value: "visible",
      });
      act(() => {
        document.dispatchEvent(new Event("visibilitychange"));
      });

      await waitFor(() => {
        expect(ttlHook.result.current.entries.get("m-ttl")).toEqual({ status: "deleted" });
      });
    } finally {
      nowSpy.mockRestore();
      if (visibilityDesc) {
        Object.defineProperty(document, "visibilityState", visibilityDesc);
      } else {
        delete (document as unknown as { visibilityState?: string }).visibilityState;
      }
    }
  });

  it("keeps entries stable across rerenders with fresh array literals when resolved IDs do not change", async () => {
    state.post.mockImplementation(async (_path: string, body: { ids: string[] }) => ({
      data: {
        items: Object.fromEntries(body.ids.map((id) => [id, okForward(id, `body-${id}`)])),
      },
      txid: null,
    }));

    let renderCount = 0;
    const { result, rerender } = renderHook(
      ({ ids }: { ids: string[] }) => {
        renderCount += 1;
        return useForwardedMessages(ids);
      },
      { initialProps: { ids: ["m-stable-b", "m-stable-a"] } },
    );

    await waitFor(() => {
      expect(result.current.entries.get("m-stable-a")).toMatchObject({
        status: "ok",
        content: "body-m-stable-a",
      });
    });

    const settledEntries = result.current.entries;
    const rendersBefore = renderCount;
    const postCallsBefore = state.post.mock.calls.length;

    // 要素順や重複が変わっても解決対象集合が同一なら再解決・state 更新を起こさない
    rerender({ ids: ["m-stable-a", "m-stable-b", "m-stable-a"] });
    expect(result.current.entries).toBe(settledEntries);
    expect(state.post).toHaveBeenCalledTimes(postCallsBefore);
    expect(renderCount).toBe(rendersBefore + 1);

    // 同じキャッシュ結果を別フックインスタンスで使っても不要な API 呼び出しが発生しない
    const secondMount = renderHook(() => useForwardedMessages(["m-stable-a"]));
    await waitFor(() => {
      expect(secondMount.result.current.entries.get("m-stable-a")).toMatchObject({
        status: "ok",
        content: "body-m-stable-a",
      });
    });
    expect(state.post).toHaveBeenCalledTimes(postCallsBefore);
  });

  it("revalidates expired entries on open visible tabs via TTL timer without visibilitychange", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    const visibilityDesc = Object.getOwnPropertyDescriptor(document, "visibilityState");
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });

    try {
      let updated = false;
      state.post.mockImplementation(async () => ({
        data: {
          items: {
            "m-open-ttl": okForward(
              "m-open-ttl",
              updated ? "updated open body" : "initial open body",
            ),
          },
        },
        txid: null,
      }));

      const { result } = renderHook(() => useForwardedMessages(["m-open-ttl"]));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
      });

      expect(result.current.entries.get("m-open-ttl")).toMatchObject({
        status: "ok",
        content: "initial open body",
      });
      expect(state.post).toHaveBeenCalledTimes(1);

      // visibilitychange を起こさずに開いたまま TTL を経過させると、タイマーで自動再検証される
      updated = true;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(FORWARD_OK_TTL_MS);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
        await Promise.resolve();
      });

      expect(result.current.entries.get("m-open-ttl")).toMatchObject({
        status: "ok",
        content: "updated open body",
      });
      expect(state.post).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
      if (visibilityDesc) {
        Object.defineProperty(document, "visibilityState", visibilityDesc);
      } else {
        delete (document as unknown as { visibilityState?: string }).visibilityState;
      }
    }
  });

  it("updates an existing hook when another hook reads and evicts the shared expired entry", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(2_000_000);
    const visibilityDesc = Object.getOwnPropertyDescriptor(document, "visibilityState");
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });

    try {
      let version = "v1";
      state.post.mockImplementation(async () => ({
        data: {
          items: {
            "m-shared": okForward("m-shared", version),
          },
        },
        txid: null,
      }));

      const hookA = renderHook(() => useForwardedMessages(["m-shared"]));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
      });
      expect(hookA.result.current.entries.get("m-shared")).toMatchObject({
        status: "ok",
        content: "v1",
      });

      // 初回書き込み完了時刻から TTL 経過後に hookB が先に期限切れエントリを削除・再取得を開始しても hookA が取り残されない
      version = "v2";
      vi.setSystemTime(Date.now() + FORWARD_OK_TTL_MS);
      const hookB = renderHook(() => useForwardedMessages(["m-shared"]));
      expect(
        peekForwardExpiresAt(
          { apiBaseUrl: state.apiBaseUrl, currentUserId: state.currentUserId },
          "m-shared",
        ),
      ).toBeUndefined();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
      });

      expect(state.post).toHaveBeenCalledTimes(2);
      expect(hookB.result.current.entries.get("m-shared")).toMatchObject({
        status: "ok",
        content: "v2",
      });
      expect(hookA.result.current.entries.get("m-shared")).toMatchObject({
        status: "ok",
        content: "v2",
      });
    } finally {
      vi.useRealTimers();
      if (visibilityDesc) {
        Object.defineProperty(document, "visibilityState", visibilityDesc);
      } else {
        delete (document as unknown as { visibilityState?: string }).visibilityState;
      }
    }
  });
});
