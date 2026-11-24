import type { ApiClient } from "./client";
import type { ForwardResolutionItem, ResolveForwardsResponse } from "./types";

export const MAX_RESOLVE_FORWARD_IDS = 100;
const BATCH_WINDOW_MS = 20;

export type ResolvedForwardEntry = ForwardResolutionItem | { status: "error" };

export interface ForwardCacheScope {
  apiBaseUrl: string;
  currentUserId: string | null;
}

interface PendingWaiter {
  resolve: (value: ResolvedForwardEntry) => void;
}

interface ScopeBucket {
  cache: Map<string, ForwardResolutionItem>;
  inflight: Map<string, Promise<ResolvedForwardEntry>>;
  pending: Map<string, PendingWaiter[]>;
  timer: ReturnType<typeof setTimeout> | null;
  waitingVisible: boolean;
  client: ApiClient | null;
}

const buckets = new Map<string, ScopeBucket>();
let visibilityListenerAttached = false;

function bucketKey(scope: ForwardCacheScope): string {
  return `${scope.apiBaseUrl}\0${scope.currentUserId ?? ""}`;
}

function getBucket(scope: ForwardCacheScope): ScopeBucket {
  const key = bucketKey(scope);
  let bucket = buckets.get(key);
  if (!bucket) {
    bucket = {
      cache: new Map(),
      inflight: new Map(),
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
  for (const bucket of buckets.values()) {
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
    const ids = chunk.map(([id]) => id);
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

    for (const [id, waiters] of chunk) {
      let value: ResolvedForwardEntry;
      if (results) {
        const resolved = results[id] ?? { status: "unavailable" };
        bucket.cache.set(id, resolved);
        value = resolved;
      } else {
        // 通信・一時エラー時はキャッシュを汚染せず、再試行できる状態として返す。
        value = { status: "error" };
      }
      bucket.inflight.delete(id);
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

/** キャッシュ済みの解決結果を返す（未解決なら `undefined`）。 */
export function cachedForwardEntry(
  scope: ForwardCacheScope,
  id: string,
): ForwardResolutionItem | undefined {
  return buckets.get(bucketKey(scope))?.cache.get(id);
}

/** 指定 ID のキャッシュだけを破棄する（通信失敗からの再試行時など）。 */
export function forgetForwardIds(scope: ForwardCacheScope, ids: readonly string[]): void {
  const bucket = buckets.get(bucketKey(scope));
  if (!bucket) return;
  for (const id of ids) {
    bucket.cache.delete(id);
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
    const cached = bucket.cache.get(id);
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
  if (visibilityListenerAttached && typeof document !== "undefined") {
    document.removeEventListener("visibilitychange", onVisibilityChange);
    visibilityListenerAttached = false;
  }
}
