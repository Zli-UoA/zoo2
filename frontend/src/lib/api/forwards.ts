import {
  getAuthSessionGeneration,
  getTokenProviderGeneration,
  subscribeAuthSessionEnded,
} from "../auth";
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

interface FlushedEntry {
  id: string;
  waiters: PendingWaiter[];
  gen: number;
  inflightPromise: Promise<ResolvedForwardEntry> | undefined;
}

interface CachedForwardEntry {
  value: ForwardResolutionItem;
  expiresAt: number;
}

interface ScopeBucket {
  scopeKey: string;
  apiBaseUrl: string;
  currentUserId: string | null;
  epoch: number;
  authSessionGeneration: number;
  tokenProviderGeneration: number;
  cache: Map<string, CachedForwardEntry>;
  inflight: Map<string, Promise<ResolvedForwardEntry>>;
  generation: Map<string, number>;
  pending: Map<string, PendingWaiter[]>;
  deferred: FlushedEntry[];
  abortControllers: Set<AbortController>;
  timer: ReturnType<typeof setTimeout> | null;
  waitingVisible: boolean;
  client: ApiClient | null;
}

const buckets = new Map<string, ScopeBucket>();
const activeUserByBaseUrl = new Map<string, string | null>();
const invalidationListeners = new Map<string, Set<(ids: readonly string[]) => void>>();
let visibilityListenerAttached = false;
let authSessionUnsubscribe: (() => void) | null = null;

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

function notifyInvalidation(scopeKey: string, ids: readonly string[]): void {
  if (ids.length === 0) return;
  for (const listener of invalidationListeners.get(scopeKey) ?? []) {
    listener(ids);
  }
}

function getFreshCacheEntry(
  scopeKey: string,
  bucket: ScopeBucket,
  id: string,
  now = Date.now(),
): ForwardResolutionItem | undefined {
  const entry = bucket.cache.get(id);
  if (!entry) return undefined;
  if (now >= entry.expiresAt) {
    bucket.cache.delete(id);
    notifyInvalidation(scopeKey, [id]);
    return undefined;
  }
  return entry.value;
}

function getBucket(scope: ForwardCacheScope): ScopeBucket {
  const key = forwardScopeKey(scope);
  let bucket = buckets.get(key);
  if (!bucket) {
    bucket = {
      scopeKey: key,
      apiBaseUrl: scope.apiBaseUrl,
      currentUserId: scope.currentUserId,
      epoch: 0,
      authSessionGeneration: getAuthSessionGeneration(),
      tokenProviderGeneration: getTokenProviderGeneration(),
      cache: new Map(),
      inflight: new Map(),
      generation: new Map(),
      pending: new Map(),
      deferred: [],
      abortControllers: new Set(),
      timer: null,
      waitingVisible: false,
      client: null,
    };
    buckets.set(key, bucket);
  }
  return bucket;
}

function settleEntriesAsError(bucket: ScopeBucket, entries: readonly FlushedEntry[]): void {
  for (const { id, waiters, inflightPromise } of entries) {
    if (inflightPromise !== undefined && bucket.inflight.get(id) === inflightPromise) {
      bucket.inflight.delete(id);
    }
    for (const waiter of waiters) {
      waiter.resolve({ status: "error" });
    }
  }
}

function discardBucket(bucket: ScopeBucket): void {
  bucket.epoch += 1;
  for (const controller of bucket.abortControllers) {
    controller.abort();
  }
  bucket.abortControllers.clear();
  if (bucket.timer !== null) {
    clearTimeout(bucket.timer);
    bucket.timer = null;
  }
  bucket.waitingVisible = false;
  if (bucket.deferred.length > 0) {
    settleEntriesAsError(bucket, bucket.deferred);
    bucket.deferred = [];
  }
  for (const waiters of bucket.pending.values()) {
    for (const waiter of waiters) {
      waiter.resolve({ status: "error" });
    }
  }
  bucket.pending.clear();
  bucket.inflight.clear();
  bucket.cache.clear();
  bucket.generation.clear();
  if (buckets.get(bucket.scopeKey) === bucket) {
    buckets.delete(bucket.scopeKey);
  }
}

function discardAllBuckets(): void {
  // ログアウト/リセット時は未認証遷移中の再フェッチを防ぐため無効化通知しない。
  for (const bucket of buckets.values()) {
    discardBucket(bucket);
  }
  activeUserByBaseUrl.clear();
}

function discardStaleSessionBucket(scopeKey: string, bucket: ScopeBucket): string[] {
  const cachedIds = isBucketActiveForBaseUrl(bucket) ? Array.from(bucket.cache.keys()) : [];
  discardBucket(bucket);
  notifyInvalidation(scopeKey, cachedIds);
  return cachedIds;
}

function ensureAuthSessionListener(): void {
  if (authSessionUnsubscribe) return;
  authSessionUnsubscribe = subscribeAuthSessionEnded((reason) => {
    if (reason === "logout" || reason === "reset") {
      discardAllBuckets();
    }
  });
}

function isBucketSessionFresh(bucket: ScopeBucket): boolean {
  return (
    bucket.authSessionGeneration === getAuthSessionGeneration() &&
    bucket.tokenProviderGeneration === getTokenProviderGeneration()
  );
}

function isBucketActiveForBaseUrl(bucket: ScopeBucket): boolean {
  return (
    !activeUserByBaseUrl.has(bucket.apiBaseUrl) ||
    activeUserByBaseUrl.get(bucket.apiBaseUrl) === bucket.currentUserId
  );
}

function isFlushStillValid(
  bucket: ScopeBucket,
  expectedEpoch: number,
  expectedAuthGen: number,
  expectedTokenGen: number,
): boolean {
  return (
    bucket.epoch === expectedEpoch &&
    buckets.get(bucket.scopeKey) === bucket &&
    bucket.authSessionGeneration === expectedAuthGen &&
    getAuthSessionGeneration() === expectedAuthGen &&
    bucket.tokenProviderGeneration === expectedTokenGen &&
    getTokenProviderGeneration() === expectedTokenGen &&
    isBucketActiveForBaseUrl(bucket)
  );
}

/** 指定スコープを現在アクティブとして登録し、同一接続先の別利用者キャッシュ・進行中バケットを破棄する。 */
export function activateForwardScope(scope: ForwardCacheScope): void {
  ensureAuthSessionListener();
  if (
    activeUserByBaseUrl.has(scope.apiBaseUrl) &&
    activeUserByBaseUrl.get(scope.apiBaseUrl) !== scope.currentUserId
  ) {
    for (const bucket of buckets.values()) {
      if (bucket.apiBaseUrl === scope.apiBaseUrl && bucket.currentUserId !== scope.currentUserId) {
        discardBucket(bucket);
      }
    }
  }
  activeUserByBaseUrl.set(scope.apiBaseUrl, scope.currentUserId);
}

function isDocumentHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}

function onVisibilityChange(): void {
  if (isDocumentHidden()) return;
  const now = Date.now();
  for (const [scopeKey, bucket] of buckets.entries()) {
    if (!isBucketSessionFresh(bucket) || !isBucketActiveForBaseUrl(bucket)) {
      discardStaleSessionBucket(scopeKey, bucket);
      continue;
    }
    const expiredIds: string[] = [];
    for (const [id, entry] of bucket.cache.entries()) {
      if (now >= entry.expiresAt) {
        bucket.cache.delete(id);
        expiredIds.push(id);
      }
    }
    notifyInvalidation(scopeKey, expiredIds);
    if (bucket.waitingVisible && (bucket.deferred.length > 0 || bucket.pending.size > 0)) {
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
  const flushEpoch = bucket.epoch;
  const flushAuthGen = bucket.authSessionGeneration;
  const flushTokenGen = bucket.tokenProviderGeneration;
  if (bucket.timer !== null) {
    clearTimeout(bucket.timer);
    bucket.timer = null;
  }
  if (!isFlushStillValid(bucket, flushEpoch, flushAuthGen, flushTokenGen)) {
    discardBucket(bucket);
    return;
  }
  if (isDocumentHidden()) {
    ensureVisibilityListener();
    bucket.waitingVisible = true;
    return;
  }
  bucket.waitingVisible = false;
  const client = bucket.client;
  if (!client || (bucket.deferred.length === 0 && bucket.pending.size === 0)) return;

  // 最初の await の前に保留済み・新規待ちの世代と inflight 所有情報を固定する。
  const deferredEntries = bucket.deferred.splice(0, bucket.deferred.length);
  const pendingEntries: FlushedEntry[] = [...bucket.pending.entries()].map(([id, waiters]) => ({
    id,
    waiters,
    gen: bucket.generation.get(id) ?? 0,
    inflightPromise: bucket.inflight.get(id),
  }));
  bucket.pending.clear();
  const entries = [...deferredEntries, ...pendingEntries];

  const controller = new AbortController();
  bucket.abortControllers.add(controller);
  try {
    for (let i = 0; i < entries.length; i += MAX_RESOLVE_FORWARD_IDS) {
      if (!isFlushStillValid(bucket, flushEpoch, flushAuthGen, flushTokenGen)) {
        settleEntriesAsError(bucket, entries.slice(i));
        discardBucket(bucket);
        return;
      }
      if (isDocumentHidden()) {
        bucket.deferred.unshift(...entries.slice(i));
        ensureVisibilityListener();
        bucket.waitingVisible = true;
        return;
      }
      const chunkMeta = entries.slice(i, i + MAX_RESOLVE_FORWARD_IDS);
      const ids = chunkMeta.map((item) => item.id);
      let results: Record<string, ForwardResolutionItem> | null = null;
      try {
        const response = await client.post<ResolveForwardsResponse>(
          "/api/messages/forwards/resolve",
          {
            ids,
          },
          { signal: controller.signal },
        );
        results = response.data?.items ?? {};
      } catch {
        results = null;
      }

      if (!isFlushStillValid(bucket, flushEpoch, flushAuthGen, flushTokenGen)) {
        settleEntriesAsError(bucket, entries.slice(i));
        discardBucket(bucket);
        return;
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
  } finally {
    bucket.abortControllers.delete(controller);
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
  const key = forwardScopeKey(scope);
  const bucket = buckets.get(key);
  if (!bucket) return undefined;
  if (!isBucketSessionFresh(bucket)) {
    discardStaleSessionBucket(key, bucket);
    return undefined;
  }
  return getFreshCacheEntry(key, bucket, id);
}

/** キャッシュを削除せずに有効な解決結果だけを参照する（期限切れ・世代不一致なら `undefined`）。 */
export function peekForwardEntry(
  scope: ForwardCacheScope,
  id: string,
  now = Date.now(),
): ForwardResolutionItem | undefined {
  const bucket = buckets.get(forwardScopeKey(scope));
  if (!bucket || !isBucketSessionFresh(bucket)) return undefined;
  const entry = bucket.cache.get(id);
  return entry && now < entry.expiresAt ? entry.value : undefined;
}

/** キャッシュを削除せずに有効期限（epoch ms）を参照する（未保持・世代不一致なら `undefined`）。 */
export function peekForwardExpiresAt(
  scope: ForwardCacheScope,
  id: string,
  now = Date.now(),
): number | undefined {
  const bucket = buckets.get(forwardScopeKey(scope));
  if (!bucket || !isBucketSessionFresh(bucket)) return undefined;
  const entry = bucket.cache.get(id);
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
  if (!isBucketSessionFresh(bucket)) {
    const discarded = discardStaleSessionBucket(key, bucket);
    const idSet = new Set(ids);
    return discarded.filter((id) => idSet.has(id));
  }
  const expiredIds: string[] = [];
  for (const id of ids) {
    const entry = bucket.cache.get(id);
    if (entry && now >= entry.expiresAt) {
      bucket.cache.delete(id);
      expiredIds.push(id);
    }
  }
  notifyInvalidation(key, expiredIds);
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
  const uniqueIds = [...new Set(ids.filter((id) => id.length > 0))];
  if (uniqueIds.length === 0) return;
  const key = forwardScopeKey(scope);
  const bucket = buckets.get(key);
  if (bucket) {
    for (const id of uniqueIds) {
      bucket.generation.set(id, (bucket.generation.get(id) ?? 0) + 1);
      bucket.inflight.delete(id);
      bucket.cache.delete(id);
    }
  }
  notifyInvalidation(key, uniqueIds);
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

  activateForwardScope(scope);
  const scopeKey = forwardScopeKey(scope);
  const existingBucket = buckets.get(scopeKey);
  if (existingBucket && !isBucketSessionFresh(existingBucket)) {
    discardStaleSessionBucket(scopeKey, existingBucket);
  }
  const bucket = getBucket(scope);
  const startEpoch = bucket.epoch;
  const startAuthGen = bucket.authSessionGeneration;
  const startTokenGen = bucket.tokenProviderGeneration;
  bucket.client = client;
  const promises: Array<[string, Promise<ResolvedForwardEntry>]> = [];
  let enqueuedNew = false;

  for (const id of uniqueIds) {
    const cached = getFreshCacheEntry(scopeKey, bucket, id);
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
  if (!isFlushStillValid(bucket, startEpoch, startAuthGen, startTokenGen)) {
    if (!isBucketSessionFresh(bucket) || !isBucketActiveForBaseUrl(bucket)) {
      discardBucket(bucket);
    }
    const aborted = new Map<string, ResolvedForwardEntry>();
    for (const id of uniqueIds) {
      aborted.set(id, { status: "error" });
    }
    return aborted;
  }
  for (let i = 0; i < promises.length; i++) {
    out.set(promises[i]![0], settled[i]!);
  }
  return out;
}

/** テスト間でキャッシュ・待機タイマー・可視状態リスナーを初期化する。 */
export function resetForwardCacheForTests(): void {
  discardAllBuckets();
  invalidationListeners.clear();
  if (visibilityListenerAttached && typeof document !== "undefined") {
    document.removeEventListener("visibilitychange", onVisibilityChange);
  }
  visibilityListenerAttached = false;
  authSessionUnsubscribe?.();
  authSessionUnsubscribe = null;
}
