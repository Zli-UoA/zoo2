import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  MAX_RESOLVE_FORWARD_IDS,
  cachedForwardEntry,
  forgetForwardIds,
  resetForwardCacheForTests,
  resolveForwardMessages,
  type ForwardCacheScope,
} from "./forwards";
import type { ForwardResolutionItem, ResolveForwardsResponse } from "./types";

function makeClient(handler: (ids: string[]) => Promise<Record<string, ForwardResolutionItem>>) {
  const post = vi.fn(async (_path: string, body: { ids: string[] }) => ({
    data: { items: await handler(body.ids) } satisfies ResolveForwardsResponse,
    txid: null,
  }));
  return {
    client: { post } as unknown as ApiClient,
    post,
  };
}

function okItem(id: string, content = `body-${id}`): ForwardResolutionItem {
  return {
    status: "ok",
    id,
    channel_id: "ch-1",
    channel_name: "general",
    is_private: false,
    is_dm: false,
    parent_id: null,
    user_id: "u-1",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    content,
    card: null,
    components: null,
    message_kind: "message",
    author: {
      id: "u-1",
      user_name: "alice",
      display_name: "Alice",
    },
    emojis: {},
    attachments: [],
    has_forwards: false,
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetForwardCacheForTests();
});

describe("resolveForwardMessages", () => {
  const scopeA: ForwardCacheScope = { apiBaseUrl: "https://a.example", currentUserId: "user-1" };
  const scopeB: ForwardCacheScope = { apiBaseUrl: "https://a.example", currentUserId: "user-2" };

  it("batches concurrent calls within 20ms, deduplicates IDs, and serves cache hits", async () => {
    vi.useFakeTimers();
    const { client, post } = makeClient(async (ids) =>
      Object.fromEntries(ids.map((id) => [id, okItem(id)])),
    );

    const p1 = resolveForwardMessages(client, scopeA, ["m1", "m2"]);
    const p2 = resolveForwardMessages(client, scopeA, ["m2", "m3"]);
    expect(post).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(20);
    const [r1, r2] = await Promise.all([p1, p2]);

    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/api/messages/forwards/resolve", {
      ids: ["m1", "m2", "m3"],
    });
    expect(r1.get("m1")?.status).toBe("ok");
    expect(r2.get("m3")?.status).toBe("ok");
    expect(cachedForwardEntry(scopeA, "m2")?.status).toBe("ok");

    // Subsequent call uses cache without issuing another request
    const r3 = await resolveForwardMessages(client, scopeA, ["m1", "m3"]);
    expect(r3.get("m1")?.status).toBe("ok");
    expect(post).toHaveBeenCalledTimes(1);
  });

  it("isolates cache entries by user scope and apiBaseUrl", async () => {
    vi.useFakeTimers();
    const { client, post } = makeClient(async (ids) =>
      Object.fromEntries(ids.map((id) => [id, okItem(id)])),
    );

    const pA = resolveForwardMessages(client, scopeA, ["m1"]);
    await vi.advanceTimersByTimeAsync(20);
    await pA;

    expect(cachedForwardEntry(scopeB, "m1")).toBeUndefined();
    const pB = resolveForwardMessages(client, scopeB, ["m1"]);
    await vi.advanceTimersByTimeAsync(20);
    await pB;

    expect(post).toHaveBeenCalledTimes(2);
  });

  it("chunks requests above MAX_RESOLVE_FORWARD_IDS (100)", async () => {
    vi.useFakeTimers();
    const { client, post } = makeClient(async (ids) =>
      Object.fromEntries(ids.map((id) => [id, { status: "unavailable" as const }])),
    );
    const ids = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 25 }, (_, i) => `m-${i}`);
    const p = resolveForwardMessages(client, scopeA, ids);
    await vi.advanceTimersByTimeAsync(20);
    const res = await p;

    expect(post).toHaveBeenCalledTimes(2);
    expect(res.size).toBe(MAX_RESOLVE_FORWARD_IDS + 25);
  });

  it("does not cache failures and supports retry via forgetForwardIds", async () => {
    vi.useFakeTimers();
    let fail = true;
    const { client, post } = makeClient(async (ids) => {
      if (fail) throw new Error("network error");
      return Object.fromEntries(ids.map((id) => [id, okItem(id)]));
    });

    const p1 = resolveForwardMessages(client, scopeA, ["m1"]);
    await vi.advanceTimersByTimeAsync(20);
    const r1 = await p1;
    expect(r1.get("m1")).toEqual({ status: "error" });
    expect(cachedForwardEntry(scopeA, "m1")).toBeUndefined();

    fail = false;
    forgetForwardIds(scopeA, ["m1"]);
    const p2 = resolveForwardMessages(client, scopeA, ["m1"]);
    await vi.advanceTimersByTimeAsync(20);
    const r2 = await p2;
    expect(r2.get("m1")?.status).toBe("ok");
    expect(post).toHaveBeenCalledTimes(2);
  });

  it("defers flush while document is hidden and flushes when visible", async () => {
    vi.useFakeTimers();
    let hidden = true;
    const listeners = new Map<string, Set<() => void>>();
    vi.stubGlobal("document", {
      get visibilityState() {
        return hidden ? "hidden" : "visible";
      },
      addEventListener(type: string, fn: () => void) {
        let set = listeners.get(type);
        if (!set) listeners.set(type, (set = new Set()));
        set.add(fn);
      },
      removeEventListener(type: string, fn: () => void) {
        listeners.get(type)?.delete(fn);
      },
    });
    const { client, post } = makeClient(async (ids) =>
      Object.fromEntries(ids.map((id) => [id, { status: "deleted" as const }])),
    );

    const p = resolveForwardMessages(client, scopeA, ["m1"]);
    await vi.advanceTimersByTimeAsync(100);
    expect(post).not.toHaveBeenCalled();

    hidden = false;
    for (const fn of listeners.get("visibilitychange") ?? []) fn();
    const res = await p;
    expect(post).toHaveBeenCalledTimes(1);
    expect(res.get("m1")).toEqual({ status: "deleted" });
    vi.unstubAllGlobals();
  });
});
