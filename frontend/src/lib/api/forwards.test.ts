import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { notifyAuthSessionEnded, resetTokenProvider, setTokenProvider } from "../auth";
import { createApiClient, type ApiClient, type FetchLike } from "./client";
import {
  FORWARD_OK_TTL_MS,
  FORWARD_UNAVAILABLE_TTL_MS,
  MAX_RESOLVE_FORWARD_IDS,
  activateForwardScope,
  cachedForwardEntry,
  evictExpiredForwardIds,
  forgetForwardIds,
  peekForwardEntry,
  peekForwardExpiresAt,
  resetForwardCacheForTests,
  resolveForwardMessages,
  subscribeForwardInvalidation,
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
  resetTokenProvider();
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
    expect(post).toHaveBeenCalledWith(
      "/api/messages/forwards/resolve",
      {
        ids: ["m1", "m2", "m3"],
      },
      { signal: expect.any(AbortSignal) },
    );
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

  it("does not pollute cache or clear newer inflight requests when forgetForwardIds runs mid-flight", async () => {
    vi.useFakeTimers();
    let callCount = 0;
    let resolveFirst!: (items: Record<string, ForwardResolutionItem>) => void;
    let resolveSecond!: (items: Record<string, ForwardResolutionItem>) => void;

    const { client, post } = makeClient(
      (ids) =>
        new Promise((resolve) => {
          callCount += 1;
          expect(ids).toEqual(["m-race"]);
          if (callCount === 1) {
            resolveFirst = resolve;
          } else {
            resolveSecond = resolve;
          }
        }),
    );

    const req1 = resolveForwardMessages(client, scopeA, ["m-race"]);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    // リクエスト 1 の応答待ち中に無効化され、続けてリクエスト 2 が開始される
    forgetForwardIds(scopeA, ["m-race"]);
    const req2 = resolveForwardMessages(client, scopeA, ["m-race"]);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(2);

    // 先に古いリクエスト 1 が返っても、キャッシュを汚染せず進行中リクエスト 2 の inflight も消さない
    resolveFirst({ "m-race": okItem("m-race", "stale") });
    await req1;
    expect(cachedForwardEntry(scopeA, "m-race")).toBeUndefined();

    // 同じタイミングの後続呼び出しは進行中のリクエスト 2 に合流し、リクエスト 3 を出さない
    const req2Follower = resolveForwardMessages(client, scopeA, ["m-race"]);
    expect(post).toHaveBeenCalledTimes(2);

    resolveSecond({ "m-race": okItem("m-race", "fresh") });
    const [res2, res2Follower] = await Promise.all([req2, req2Follower]);
    expect(res2.get("m-race")).toMatchObject({ status: "ok", content: "fresh" });
    expect(res2Follower.get("m-race")).toMatchObject({ status: "ok", content: "fresh" });
    expect(cachedForwardEntry(scopeA, "m-race")).toMatchObject({
      status: "ok",
      content: "fresh",
    });
  });

  it("peeks expiry non-destructively and notifies only expired IDs via evictExpiredForwardIds", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(2_000_000);
    const { client } = makeClient(async () => ({
      "m-soon": { status: "unavailable" },
      "m-later": okItem("m-later", "live"),
    }));

    const p = resolveForwardMessages(client, scopeA, ["m-soon", "m-later"]);
    await vi.advanceTimersByTimeAsync(20);
    await p;

    const listener = vi.fn();
    const unsubscribe = subscribeForwardInvalidation(scopeA, listener);
    try {
      await vi.advanceTimersByTimeAsync(FORWARD_UNAVAILABLE_TTL_MS);

      // peekForwardEntry は期限切れでもキャッシュ自体を削除せず、evictExpiredForwardIds まで保持する
      expect(peekForwardEntry(scopeA, "m-soon")).toBeUndefined();
      expect(peekForwardExpiresAt(scopeA, "m-soon")).toBeDefined();
      expect(peekForwardEntry(scopeA, "m-later")).toMatchObject({ status: "ok", content: "live" });

      const evicted = evictExpiredForwardIds(scopeA, ["m-soon", "m-later"]);
      expect(evicted).toEqual(["m-soon"]);
      expect(listener).toHaveBeenCalledTimes(1);
      expect(listener).toHaveBeenCalledWith(["m-soon"]);
      expect(peekForwardExpiresAt(scopeA, "m-soon")).toBeUndefined();
      expect(peekForwardEntry(scopeA, "m-later")).toMatchObject({ status: "ok", content: "live" });

      expect(evictExpiredForwardIds(scopeA, ["m-soon", "m-later"])).toEqual([]);
      expect(listener).toHaveBeenCalledTimes(1);
    } finally {
      unsubscribe();
    }
  });

  it("snapshots all chunk ownership before first await so invalidating id #101 mid-flush cannot evict newer inflight", async () => {
    vi.useFakeTimers();
    const pendingResolvers: Array<(items: Record<string, ForwardResolutionItem>) => void> = [];
    const seenRequests: string[][] = [];

    const { client, post } = makeClient(
      (ids) =>
        new Promise((resolve) => {
          seenRequests.push(ids);
          pendingResolvers.push(resolve);
        }),
    );

    const manyIds = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 1 }, (_, i) => `m-${i + 1}`);
    const targetId = `m-${MAX_RESOLVE_FORWARD_IDS + 1}`;

    // リクエスト A (101 件) のフラッシュ開始: 先頭 100 件チャンク (index 0) が応答待ちに入る
    const reqA = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    // 1 チャンク目の完了前に 101 件目 (targetId) を無効化し、新しいリクエスト B を開始する
    forgetForwardIds(scopeA, [targetId]);
    const reqB = resolveForwardMessages(client, scopeA, [targetId]);
    await vi.advanceTimersByTimeAsync(20);
    // リクエスト B が targetId のポスト (index 1) を送信済み
    expect(post).toHaveBeenCalledTimes(2);
    expect(seenRequests[1]).toEqual([targetId]);

    // ここでリクエスト A のチャンク 1 が完了し、A のチャンク 2 (index 2) が送信される
    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `chunk1-${id}`)])),
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(3);
    expect(seenRequests[2]).toEqual([targetId]);

    // 先にリクエスト A のチャンク 2 (stale) が返っても、キャッシュ汚染も B の inflight 削除もしない
    pendingResolvers[2]!({ [targetId]: okItem(targetId, "stale-101") });
    await reqA;
    expect(cachedForwardEntry(scopeA, targetId)).toBeUndefined();

    // 後続の解決呼び出しはリクエスト B に合流し、追加の POST を発生させない
    const reqBFollower = resolveForwardMessages(client, scopeA, [targetId]);
    expect(post).toHaveBeenCalledTimes(3);

    pendingResolvers[1]!({ [targetId]: okItem(targetId, "fresh-101") });
    const [resB, resFollower] = await Promise.all([reqB, reqBFollower]);
    expect(resB.get(targetId)).toMatchObject({ status: "ok", content: "fresh-101" });
    expect(resFollower.get(targetId)).toMatchObject({ status: "ok", content: "fresh-101" });
    expect(cachedForwardEntry(scopeA, targetId)).toMatchObject({
      status: "ok",
      content: "fresh-101",
    });
  });

  it("notifies invalidation listeners when getFreshCacheEntry deletes an expired entry", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(3_000_000);
    const { client } = makeClient(async () => ({
      "m-exp": okItem("m-exp", "initial"),
    }));

    const p = resolveForwardMessages(client, scopeA, ["m-exp"]);
    await vi.advanceTimersByTimeAsync(20);
    await p;

    const listener = vi.fn();
    const unsubscribe = subscribeForwardInvalidation(scopeA, listener);
    try {
      await vi.advanceTimersByTimeAsync(FORWARD_OK_TTL_MS);
      expect(cachedForwardEntry(scopeA, "m-exp")).toBeUndefined();
      expect(listener).toHaveBeenCalledTimes(1);
      expect(listener).toHaveBeenCalledWith(["m-exp"]);
    } finally {
      unsubscribe();
    }
  });

  it("defers remaining chunks if tab becomes hidden mid-flush and resumes on visibilitychange", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, Set<() => void>>();
    let hidden = false;
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

    const pendingResolvers: Array<(items: Record<string, ForwardResolutionItem>) => void> = [];
    const seenRequests: string[][] = [];
    const { client, post } = makeClient(
      (ids) =>
        new Promise((resolve) => {
          seenRequests.push(ids);
          pendingResolvers.push(resolve);
        }),
    );

    const manyIds = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 1 }, (_, i) => `m-vis-${i + 1}`);
    const lastId = `m-vis-${MAX_RESOLVE_FORWARD_IDS + 1}`;
    const req = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    // チャンク 1 の応答待ち中にタブが非表示になると、チャンク 2 は即送信せず保留する
    hidden = true;
    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `body-${id}`)])),
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(1);

    // タブが可視に復帰すると保留していたチャンク 2 が送信される
    hidden = false;
    for (const fn of listeners.get("visibilitychange") ?? []) fn();
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(2);
    expect(seenRequests[1]).toEqual([lastId]);

    pendingResolvers[1]!({ [lastId]: okItem(lastId, "resumed-last") });
    const res = await req;
    expect(res.get(lastId)).toMatchObject({ status: "ok", content: "resumed-last" });
  });

  it("notifies all deduped requested IDs in forgetForwardIds even when some exist only in local error state", async () => {
    vi.useFakeTimers();
    const { client } = makeClient(async () => ({
      "m-cached": okItem("m-cached", "cached body"),
    }));
    const p = resolveForwardMessages(client, scopeA, ["m-cached"]);
    await vi.advanceTimersByTimeAsync(20);
    await p;

    const listener = vi.fn();
    const unsubscribe = subscribeForwardInvalidation(scopeA, listener);
    try {
      forgetForwardIds(scopeA, ["m-cached", "m-local-error", "m-cached", ""]);
      expect(listener).toHaveBeenCalledTimes(1);
      expect(listener).toHaveBeenCalledWith(["m-cached", "m-local-error"]);
    } finally {
      unsubscribe();
    }
  });

  it("registers visibility listener and resumes deferred chunks even when first chunk rejects while hidden", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, Set<() => void>>();
    let hidden = false;
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

    const deferredCalls: Array<{
      resolve: (items: Record<string, ForwardResolutionItem>) => void;
      reject: (reason?: unknown) => void;
    }> = [];
    const seenRequests: string[][] = [];
    const { client, post } = makeClient(
      (ids) =>
        new Promise((resolve, reject) => {
          seenRequests.push(ids);
          deferredCalls.push({ resolve, reject });
        }),
    );

    const manyIds = Array.from(
      { length: MAX_RESOLVE_FORWARD_IDS + 1 },
      (_, i) => `m-err-vis-${i + 1}`,
    );
    const lastId = `m-err-vis-${MAX_RESOLVE_FORWARD_IDS + 1}`;
    const req = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    // 先頭チャンク失敗時もキャッシュ保存側の listener 登録には頼らず、保留分岐で visibilitychange を購読する
    hidden = true;
    deferredCalls[0]!.reject(new Error("transient failure"));
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(1);
    expect(listeners.get("visibilitychange")?.size).toBe(1);

    hidden = false;
    for (const fn of listeners.get("visibilitychange") ?? []) fn();
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(2);
    expect(seenRequests[1]).toEqual([lastId]);

    deferredCalls[1]!.resolve({ [lastId]: okItem(lastId, "recovered-after-fail") });
    const res = await req;
    expect(res.get(manyIds[0]!)).toEqual({ status: "error" });
    expect(res.get(lastId)).toMatchObject({ status: "ok", content: "recovered-after-fail" });
  });

  function makeDeferredPost() {
    const pendingResolvers: Array<(items: Record<string, ForwardResolutionItem>) => void> = [];
    const seenRequests: string[][] = [];
    const { client, post } = makeClient(
      (ids) =>
        new Promise((resolve) => {
          seenRequests.push(ids);
          pendingResolvers.push(resolve);
        }),
    );
    return { client, post, pendingResolvers, seenRequests };
  }

  it("aborts remaining chunks, clears cache, and resolves callers with error on logout mid-flush", async () => {
    vi.useFakeTimers();
    const { client, post, pendingResolvers, seenRequests } = makeDeferredPost();
    const manyIds = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 1 }, (_, i) => `m-out-${i + 1}`);
    const lastId = `m-out-${MAX_RESOLVE_FORWARD_IDS + 1}`;

    const req = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    notifyAuthSessionEnded("logout");
    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `leak-${id}`)])),
    );
    const res = await req;
    expect(post).toHaveBeenCalledTimes(1);
    expect(res.get(manyIds[0]!)).toEqual({ status: "error" });
    expect(res.get(lastId)).toEqual({ status: "error" });
    expect(cachedForwardEntry(scopeA, manyIds[0]!)).toBeUndefined();
    expect(cachedForwardEntry(scopeA, lastId)).toBeUndefined();
  });

  it("aborts remaining chunks of previous user without caching or leaking responses when switching user scope", async () => {
    vi.useFakeTimers();
    const { client, post, pendingResolvers, seenRequests } = makeDeferredPost();
    const manyIds = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 1 }, (_, i) => `m-sw-${i + 1}`);
    const lastId = `m-sw-${MAX_RESOLVE_FORWARD_IDS + 1}`;

    const reqA = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    activateForwardScope(scopeB);
    const reqB = resolveForwardMessages(client, scopeB, ["m-user-b"]);
    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `alice-${id}`)])),
    );
    const resA = await reqA;
    expect(resA.get(manyIds[0]!)).toEqual({ status: "error" });
    expect(resA.get(lastId)).toEqual({ status: "error" });
    expect(cachedForwardEntry(scopeA, manyIds[0]!)).toBeUndefined();

    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(2);
    expect(seenRequests[1]).toEqual(["m-user-b"]);
    pendingResolvers[1]!({ "m-user-b": okItem("m-user-b", "bob-ok") });
    const resB = await reqB;
    expect(resB.get("m-user-b")).toMatchObject({ status: "ok", content: "bob-ok" });
  });

  it("aborts flush and discards response when tokenProvider changes mid-flush", async () => {
    vi.useFakeTimers();
    setTokenProvider(() => "token-user-1");
    const { client, post, pendingResolvers, seenRequests } = makeDeferredPost();
    const manyIds = Array.from({ length: MAX_RESOLVE_FORWARD_IDS + 1 }, (_, i) => `m-tok-${i + 1}`);
    const lastId = `m-tok-${MAX_RESOLVE_FORWARD_IDS + 1}`;

    const req = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    setTokenProvider(() => "token-user-2");
    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `wrong-${id}`)])),
    );
    const res = await req;
    expect(post).toHaveBeenCalledTimes(1);
    expect(res.get(manyIds[0]!)).toEqual({ status: "error" });
    expect(res.get(lastId)).toEqual({ status: "error" });
    expect(cachedForwardEntry(scopeA, manyIds[0]!)).toBeUndefined();
    expect(cachedForwardEntry(scopeA, lastId)).toBeUndefined();
  });

  it("notifies invalidation listeners on visibilitychange when tokenProvider changed and stays silent on logout", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, Set<() => void>>();
    let hidden = false;
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

    setTokenProvider(() => "token-1");
    const { client } = makeClient(async (ids) =>
      Object.fromEntries(ids.map((id) => [id, okItem(id, `body-${id}`)])),
    );
    const req = resolveForwardMessages(client, scopeA, ["m-rot-1", "m-rot-2"]);
    await vi.advanceTimersByTimeAsync(20);
    await req;

    const listener = vi.fn();
    const unsubscribe = subscribeForwardInvalidation(scopeA, listener);
    try {
      setTokenProvider(() => "token-2");
      expect(peekForwardEntry(scopeA, "m-rot-1")).toBeUndefined();
      expect(peekForwardExpiresAt(scopeA, "m-rot-1")).toBeUndefined();
      expect(listener).not.toHaveBeenCalled();

      hidden = false;
      for (const fn of listeners.get("visibilitychange") ?? []) fn();
      expect(listener).toHaveBeenCalledTimes(1);
      expect(listener).toHaveBeenCalledWith(["m-rot-1", "m-rot-2"]);

      // ログアウト時の全体破棄では再解決を促す通知を飛ばさない
      const req2 = resolveForwardMessages(client, scopeA, ["m-out-silent"]);
      await vi.advanceTimersByTimeAsync(20);
      await req2;
      listener.mockClear();
      notifyAuthSessionEnded("logout");
      expect(listener).not.toHaveBeenCalled();
    } finally {
      unsubscribe();
    }
  });

  it("never invokes fetch via real createApiClient when auth or user changes while getToken is pending", async () => {
    vi.useFakeTimers();
    for (const mode of ["logout", "scope", "provider"] as const) {
      resetForwardCacheForTests();
      let resolveToken!: (token: string | null) => void;
      let tokenRequested!: () => void;
      const tokenWait = new Promise<void>((resolve) => {
        tokenRequested = resolve;
      });
      setTokenProvider(
        () =>
          new Promise<string | null>((resolve) => {
            resolveToken = resolve;
            tokenRequested();
          }),
      );
      const fetchMock = vi.fn<FetchLike>(
        async () =>
          new Response(JSON.stringify({ data: { items: {} }, txid: null }), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          }),
      );
      const realClient = createApiClient({ baseUrl: "https://a.example", fetch: fetchMock });

      const targetId = `m-pending-tok-${mode}`;
      const req = resolveForwardMessages(realClient, scopeA, [targetId]);
      await vi.advanceTimersByTimeAsync(20);
      await tokenWait;
      expect(fetchMock).not.toHaveBeenCalled();

      if (mode === "logout") {
        notifyAuthSessionEnded("logout");
      } else if (mode === "scope") {
        activateForwardScope(scopeB);
      } else {
        setTokenProvider(() => "new-user-token");
      }

      resolveToken("old-user-token");
      const res = await req;
      expect(fetchMock).not.toHaveBeenCalled();
      expect(res.get(targetId)).toEqual({ status: "error" });
      expect(cachedForwardEntry(scopeA, targetId)).toBeUndefined();
    }
  });

  it("discards chunk 1 results too if tokenProvider changes while awaiting chunk 2", async () => {
    vi.useFakeTimers();
    setTokenProvider(() => "token-1");
    const { client, post, pendingResolvers, seenRequests } = makeDeferredPost();
    const manyIds = Array.from(
      { length: MAX_RESOLVE_FORWARD_IDS + 1 },
      (_, i) => `m-mchunk-${i + 1}`,
    );
    const lastId = `m-mchunk-${MAX_RESOLVE_FORWARD_IDS + 1}`;

    const req = resolveForwardMessages(client, scopeA, manyIds);
    await vi.advanceTimersByTimeAsync(20);
    expect(post).toHaveBeenCalledTimes(1);

    pendingResolvers[0]!(
      Object.fromEntries(seenRequests[0]!.map((id) => [id, okItem(id, `chunk1-${id}`)])),
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(post).toHaveBeenCalledTimes(2);

    setTokenProvider(() => "token-2");
    pendingResolvers[1]!({ [lastId]: okItem(lastId, "chunk2-stale") });

    const res = await req;
    expect(res.get(manyIds[0]!)).toEqual({ status: "error" });
    expect(res.get(lastId)).toEqual({ status: "error" });
    expect(cachedForwardEntry(scopeA, manyIds[0]!)).toBeUndefined();
    expect(cachedForwardEntry(scopeA, lastId)).toBeUndefined();
  });
});
