import type { ApiClient } from "./client";
import type { ForwardResolutionItem, ResolveForwardsResponse } from "./types";

export const MAX_RESOLVE_FORWARD_IDS = 100;
const BATCH_WINDOW_MS = 20;
export const FORWARD_OK_TTL_MS = 60_000;
export const FORWARD_UNAVAILABLE_TTL_MS = 30_000;
export const FORWARD_DELETED_TTL_MS = 300_000;

export type ResolvedForwardEntry = ForwardResolutionItem | { status: "error" };

export interface ForwardCacheScope {
  apiBaseUrl: string;
  currentUserId: string | null;
}

interface PendingWaiter {
  resolve: (value: ResolvedForwardEntry) => void;
}

interface CachedForwardEntry {
  value: ForwardResolutionItem;
  expiresAt: number;
}

interface ScopeBucket {
  cache: Map<string, CachedForwardEntry>;
  inflight: Map<string, Promise<ResolvedForwardEntry>>;
  generation: Map<string, number>;
  pending: Map<string, PendingWaiter[]>;
  timer: ReturnType<typeof setTimeout> | null;
  waitingVisible: boolean;
  client: ApiClient | null;
}

const buckets = new Map<string, ScopeBucket>();
const invalidationListeners = new Map<string, Set<(ids: readonly string[]) => void>>();
let visibilityListenerAttached = false;

export function forwardScopeKey(scope: ForwardCacheScope): string {
  return `${scope.apiBaseUrl}\0${scope.currentUserId ?? ""}`;
}

function ttlForItem(item: ForwardResolutionItem): number {
  switch (item.status) {
    case "ok":
      return FORWARD_OK_TTL_MS;
    case "unavailable":
      return FORWARD_UNAVAILABLE_TTL_MS;
    case "deleted":
      return FORWARD_DELETED_TTL_MS;
  }
}

function getFreshCacheEntry(
  bucket: ScopeBucket,
  id: string,
  now = Date.now(),
): ForwardResolutionItem | undefined {
  const entry = bucket.cache.get(id);
  if (!entry) return undefined;
  if (now >= entry.expiresAt) {
    bucket.cache.delete(id);
    return undefined;
  }
  return entry.value;
}

function getBucket(scope: ForwardCacheScope): ScopeBucket {
  const key = forwardScopeKey(scope);
  let bucket = buckets.get(key);
  if (!bucket) {
    bucket = {
      cache: new Map(),
      inflight: new Map(),
      generation: new Map(),
      pending: new Map(),
      timer: null,
      waitingVisible: false,
      client: null,
    };
    buckets.set(key, bucket);
  }
  return bucket;
}

function isDocumentHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}

function onVisibilityChange(): void {
  if (isDocumentHidden()) return;
  const now = Date.now();
  for (const [scopeKey, bucket] of buckets.entries()) {
    const expiredIds: string[] = [];
    for (const [id, entry] of bucket.cache.entries()) {
      if (now >= entry.expiresAt) {
        bucket.cache.delete(id);
        expiredIds.push(id);
      }
    }
    if (expiredIds.length > 0) {
      for (const listener of invalidationListeners.get(scopeKey) ?? []) {
        listener(expiredIds);
      }
    }
    if (bucket.waitingVisible && bucket.pending.size > 0) {
      bucket.waitingVisible = false;
      void flushBucket(bucket);
    }
  }
}

function ensureVisibilityListener(): void {
  if (visibilityListenerAttached || typeof document === "undefined") return;
  document.addEventListener("visibilitychange", onVisibilityChange);
  visibilityListenerAttached = true;
}

async function flushBucket(bucket: ScopeBucket): Promise<void> {
  if (bucket.timer !== null) {
    clearTimeout(bucket.timer);
    bucket.timer = null;
  }
  if (isDocumentHidden()) {
    ensureVisibilityListener();
    bucket.waitingVisible = true;
    return;
  }
  bucket.waitingVisible = false;
  const client = bucket.client;
  if (!client || bucket.pending.size === 0) return;

  const entries = [...bucket.pending.entries()];
  bucket.pending.clear();

  for (let i = 0; i < entries.length; i += MAX_RESOLVE_FORWARD_IDS) {
    const chunk = entries.slice(i, i + MAX_RESOLVE_FORWARD_IDS);
    const chunkMeta = chunk.map(([id, waiters]) => ({
      id,
      waiters,
      gen: bucket.generation.get(id) ?? 0,
      inflightPromise: bucket.inflight.get(id),
    }));
    const ids = chunkMeta.map((item) => item.id);
    let results: Record<string, ForwardResolutionItem> | null = null;
    try {
      const response = await client.post<ResolveForwardsResponse>(
        "/api/messages/forwards/resolve",
        {
          ids,
        },
      );
      results = response.data?.items ?? {};
    } catch {
      results = null;
    }

    for (const { id, waiters, gen, inflightPromise } of chunkMeta) {
      let value: ResolvedForwardEntry;
      const isCurrentGeneration = (bucket.generation.get(id) ?? 0) === gen;
      if (results) {
        const resolved = results[id] ?? { status: "unavailable" };
        if (isCurrentGeneration) {
          bucket.cache.set(id, {
            value: resolved,
            expiresAt: Date.now() + ttlForItem(resolved),
          });
          ensureVisibilityListener();
        }
        value = resolved;
      } else {
        // 通信・一時エラー時はキャッシュを汚染せず、再試行できる状態として返す。
        value = { status: "error" };
      }
      if (inflightPromise !== undefined && bucket.inflight.get(id) === inflightPromise) {
        bucket.inflight.delete(id);
      }
      for (const waiter of waiters) {
        waiter.resolve(value);
      }
    }
  }
}

function scheduleFlush(bucket: ScopeBucket): void {
  if (isDocumentHidden()) {
    ensureVisibilityListener();
    bucket.waitingVisible = true;
    return;
  }
  if (bucket.timer !== null) return;
  bucket.timer = setTimeout(() => {
    bucket.timer = null;
    void flushBucket(bucket);
  }, BATCH_WINDOW_MS);
}

/** キャッシュ済みの解決結果を返す（未解決または期限切れなら `undefined`）。 */
export function cachedForwardEntry(
  scope: ForwardCacheScope,
  id: string,
): ForwardResolutionItem | undefined {
  const bucket = buckets.get(forwardScopeKey(scope));
  return bucket ? getFreshCacheEntry(bucket, id) : undefined;
}

/** キャッシュを削除せずに有効な解決結果だけを参照する（期限切れなら `undefined`）。 */
export function peekForwardEntry(
  scope: ForwardCacheScope,
  id: string,
  now = Date.now(),
): ForwardResolutionItem | undefined {
  const entry = buckets.get(forwardScopeKey(scope))?.cache.get(id);
  return entry && now < entry.expiresAt ? entry.value : undefined;
}

/** キャッシュを削除せずに有効期限（epoch ms）を参照する（未保持なら `undefined`）。 */
export function peekForwardExpiresAt(
  scope: ForwardCacheScope,
  id: string,
  now = Date.now(),
): number | undefined {
  const entry = buckets.get(forwardScopeKey(scope))?.cache.get(id);
  if (!entry) return undefined;
  return now >= entry.expiresAt ? now : entry.expiresAt;
}

/** 指定 ID 群のうち有効期限切れのエントリだけを破棄し、購読者へ通知する。 */
export function evictExpiredForwardIds(
  scope: ForwardCacheScope,
  ids: readonly string[],
  now = Date.now(),
): string[] {
  const key = forwardScopeKey(scope);
  const bucket = buckets.get(key);
  if (!bucket) return [];
  const expiredIds: string[] = [];
  for (const id of ids) {
    const entry = bucket.cache.get(id);
    if (entry && now >= entry.expiresAt) {
      bucket.cache.delete(id);
      expiredIds.push(id);
    }
  }
  if (expiredIds.length > 0) {
    for (const listener of invalidationListeners.get(key) ?? []) {
      listener(expiredIds);
    }
  }
  return expiredIds;
}

/** 指定 ID のキャッシュ破棄イベントを購読する（編集・削除後の即時再解決用）。 */
export function subscribeForwardInvalidation(
  scope: ForwardCacheScope,
  listener: (ids: readonly string[]) => void,
): () => void {
  const key = forwardScopeKey(scope);
  let set = invalidationListeners.get(key);
  if (!set) {
    set = new Set();
    invalidationListeners.set(key, set);
  }
  set.add(listener);
  return () => {
    const current = invalidationListeners.get(key);
    if (!current) return;
    current.delete(listener);
    if (current.size === 0) {
      invalidationListeners.delete(key);
    }
  };
}

/** 指定 ID のキャッシュだけを破棄する（通信失敗からの再試行や編集・削除時）。 */
export function forgetForwardIds(scope: ForwardCacheScope, ids: readonly string[]): void {
  const key = forwardScopeKey(scope);
  const bucket = buckets.get(key);
  const deleted: string[] = [];
  if (bucket) {
    for (const id of ids) {
      bucket.generation.set(id, (bucket.generation.get(id) ?? 0) + 1);
      const removedInflight = bucket.inflight.delete(id);
      const removedCache = bucket.cache.delete(id);
      if (removedCache || removedInflight) {
        deleted.push(id);
      }
    }
  }
  const target = deleted.length > 0 ? deleted : ids;
  if (target.length > 0) {
    for (const listener of invalidationListeners.get(key) ?? []) {
      listener(target);
    }
  }
}

/**
 * 転送元メッセージ ID 群を解決する。同一スコープ内では 20ms ウィンドウで
 * 最大 100 件ずつバッチ化し、進行中および解決済みの ID を重複排除する。
 */
export async function resolveForwardMessages(
  client: ApiClient,
  scope: ForwardCacheScope,
  ids: readonly string[],
): Promise<Map<string, ResolvedForwardEntry>> {
  const uniqueIds = [...new Set(ids.filter((id) => id.length > 0))];
  const out = new Map<string, ResolvedForwardEntry>();
  if (uniqueIds.length === 0) return out;

  const bucket = getBucket(scope);
  bucket.client = client;
  const promises: Array<[string, Promise<ResolvedForwardEntry>]> = [];
  let enqueuedNew = false;

  for (const id of uniqueIds) {
    const cached = getFreshCacheEntry(bucket, id);
    if (cached) {
      out.set(id, cached);
      continue;
    }
    const existing = bucket.inflight.get(id);
    if (existing) {
      promises.push([id, existing]);
      continue;
    }
    const promise = new Promise<ResolvedForwardEntry>((resolve) => {
      const list = bucket.pending.get(id);
      if (list) {
        list.push({ resolve });
      } else {
        bucket.pending.set(id, [{ resolve }]);
      }
    });
    bucket.inflight.set(id, promise);
    promises.push([id, promise]);
    enqueuedNew = true;
  }

  if (enqueuedNew) {
    scheduleFlush(bucket);
  }

  const settled = await Promise.all(promises.map(([, p]) => p));
  for (let i = 0; i < promises.length; i++) {
    out.set(promises[i]![0], settled[i]!);
  }
  return out;
}

/** テスト間でキャッシュ・待機タイマー・可視状態リスナーを初期化する。 */
export function resetForwardCacheForTests(): void {
  for (const bucket of buckets.values()) {
    if (bucket.timer !== null) {
      clearTimeout(bucket.timer);
      bucket.timer = null;
    }
    for (const waiters of bucket.pending.values()) {
      for (const waiter of waiters) {
        waiter.resolve({ status: "error" });
      }
    }
    bucket.pending.clear();
    bucket.inflight.clear();
    bucket.cache.clear();
  }
  buckets.clear();
  invalidationListeners.clear();
  if (visibilityListenerAttached && typeof document !== "undefined") {
    document.removeEventListener("visibilitychange", onVisibilityChange);
  }
  visibilityListenerAttached = false;
}
