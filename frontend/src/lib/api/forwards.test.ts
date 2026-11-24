import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  FORWARD_OK_TTL_MS,
  FORWARD_UNAVAILABLE_TTL_MS,
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
  vi.unstubAllGlobals();
});

describe("resolveForwardMessages", () => {
  const scopeA: ForwardCacheScope = { apiBaseUrl: "https://a.example", currentUserId: "user-1" };
  const scopeB: ForwardCacheScope = { apiBaseUrl: "https://a.example", currentUserId: "user-2" };
  const scopeC: ForwardCacheScope = { apiBaseUrl: "https://b.example", currentUserId: "user-1" };

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

    expect(cachedForwardEntry(scopeC, "m1")).toBeUndefined();
    const pC = resolveForwardMessages(client, scopeC, ["m1"]);
    await vi.advanceTimersByTimeAsync(20);
    await pC;

    expect(post).toHaveBeenCalledTimes(3);
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
    const sentIds: string[] = [];
    for (const [, body] of post.mock.calls) {
      expect(body.ids.length).toBeLessThanOrEqual(MAX_RESOLVE_FORWARD_IDS);
      sentIds.push(...body.ids);
    }
    expect(sentIds.sort()).toEqual([...ids].sort());
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

  it("registers visibility listener when document becomes hidden before scheduled timer fires", async () => {
    vi.useFakeTimers();
    let hidden = false;
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
      Object.fromEntries(ids.map((id) => [id, okItem(id)])),
    );

    const p = resolveForwardMessages(client, scopeA, ["m-vis"]);
    // タイマー発火前にタブが非表示へ遷移したケース
    hidden = true;
    await vi.advanceTimersByTimeAsync(20);
    expect(post).not.toHaveBeenCalled();

    hidden = false;
    for (const fn of listeners.get("visibilitychange") ?? []) fn();
    const res = await p;
    expect(post).toHaveBeenCalledTimes(1);
    expect(res.get("m-vis")?.status).toBe("ok");
    vi.unstubAllGlobals();
  });

  it("expires cached unavailable and ok entries after their TTLs", async () => {
    vi.useFakeTimers();
    let phase: "unavailable" | "ok1" | "ok2" = "unavailable";
    const { client, post } = makeClient(async (ids) => {
      if (phase === "unavailable") {
        return Object.fromEntries(ids.map((id) => [id, { status: "unavailable" as const }]));
      }
      return Object.fromEntries(
        ids.map((id) => [id, okItem(id, phase === "ok1" ? "initial" : "edited")]),
      );
    });

    const p1 = resolveForwardMessages(client, scopeA, ["m-ttl"]);
    await vi.advanceTimersByTimeAsync(20);
    const r1 = await p1;
    expect(r1.get("m-ttl")?.status).toBe("unavailable");
    expect(post).toHaveBeenCalledTimes(1);

    // unavailable は TTL (30s) 内はキャッシュヒットし、経過後は再取得する
    phase = "ok1";
    await vi.advanceTimersByTimeAsync(FORWARD_UNAVAILABLE_TTL_MS - 1_000);
    const rHit = await resolveForwardMessages(client, scopeA, ["m-ttl"]);
    expect(rHit.get("m-ttl")?.status).toBe("unavailable");
    expect(post).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(cachedForwardEntry(scopeA, "m-ttl")).toBeUndefined();
    const p2 = resolveForwardMessages(client, scopeA, ["m-ttl"]);
    await vi.advanceTimersByTimeAsync(20);
    const r2 = await p2;
    expect(r2.get("m-ttl")).toMatchObject({ status: "ok", content: "initial" });
    expect(post).toHaveBeenCalledTimes(2);

    // ok も TTL (60s) 経過後に更新内容を再取得する
    phase = "ok2";
    await vi.advanceTimersByTimeAsync(FORWARD_OK_TTL_MS);
    const p3 = resolveForwardMessages(client, scopeA, ["m-ttl"]);
    await vi.advanceTimersByTimeAsync(20);
    const r3 = await p3;
    expect(r3.get("m-ttl")).toMatchObject({ status: "ok", content: "edited" });
    expect(post).toHaveBeenCalledTimes(3);
  });
});
