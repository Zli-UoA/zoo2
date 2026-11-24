import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import {
  FORWARD_OK_TTL_MS,
  forgetForwardIds,
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
});
