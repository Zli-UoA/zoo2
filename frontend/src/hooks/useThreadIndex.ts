import { useMemo, useSyncExternalStore } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { THREADS_CHANGED, threadRevision } from "../lib/electric/threads";

export interface ThreadSummary {
  id: string;
  channel_id: string;
  content: string;
  thread_archive_at: string | null;
  reply_count: number;
}

export interface ThreadPage {
  items: ThreadSummary[];
  next_cursor: string | null;
}
// 取得開始とローカルでの状態検知を、共有キャッシュ全体で同じ順序に並べる。
let sequence = 0;
const requestSequences = new WeakMap<ThreadSummary, number>();
export function nextThreadIndexSequence() {
  return ++sequence;
}
export function threadSummaryRequestSequence(thread: ThreadSummary) {
  return requestSequences.get(thread) ?? 0;
}

const EMPTY_PAGE: ThreadPage = { items: [], next_cursor: null };
type Result = ThreadPage & { key: string; isLoading: boolean; isError: boolean };

/** 同じアカウント・範囲の一覧、ページ取得、更新要求を共有する。 */
const stores = new WeakMap<object, Map<string, ReturnType<typeof createIndex>>>();
function createIndex(
  config: Parameters<typeof createConfiguredApiClient>[0],
  userId: string | null,
  channelId: string | undefined,
  archived: boolean,
  key: string,
) {
  const client = createConfiguredApiClient(config);
  let result: Result | null = null;
  const observed = new Map<string, { thread: ThreadSummary; sequence: number }>();
  let fetchedAt = 0;
  let observedRevision = -1;
  let cursor: string | null = null;
  let hasAdditionalPages = false;
  let failedRequest: { append: boolean; cursor: string | null } | null = null;
  const listeners = new Set<() => void>();
  let stop: (() => void) | undefined;
  let loadMore = () => {};
  const setResult = (update: (current: Result | null) => Result | null) => {
    result = update(result);
    listeners.forEach((listener) => listener());
  };
  const start = () => {
    let active = true;
    let inFlight = false;
    let pending: "refresh" | "append" | null = null;
    let generation = 0;
    let controller: AbortController | undefined;
    const refresh = async (append = false, before = cursor) => {
      if (document.visibilityState === "hidden") return;
      if (inFlight) {
        pending = !append || pending === "refresh" ? "refresh" : "append";
        return;
      }
      inFlight = true;
      const request = new AbortController();
      controller = request;
      const signal = AbortSignal.any([request.signal, AbortSignal.timeout(10_000)]);
      const requestedSequence = nextThreadIndexSequence();
      const started = generation;
      const startedRevision = threadRevision();
      const params = new URLSearchParams();
      if (channelId) params.set("channel_id", channelId);
      if (archived) params.set("archived", "true");
      const rebuild = !append && hasAdditionalPages;
      const rangeEnd = rebuild ? before : null;
      if (rangeEnd) params.set("through", rangeEnd);
      if (append && before) params.set("before", before);
      const query = params.size ? `?${params}` : "";
      setResult((current) => ({
        ...(current?.key === key ? current : EMPTY_PAGE),
        key,
        isLoading: true,
        isError: false,
      }));
      try {
        let page = await client.get<ThreadPage>(`/api/threads${query}`, { signal });
        const items = [...page.items];
        const seenCursors = new Set<string>();
        while (rebuild && page.next_cursor) {
          if (!active || started !== generation || signal.aborted) return;
          if (seenCursors.has(page.next_cursor)) throw new Error("Repeated thread cursor");
          seenCursors.add(page.next_cursor);
          params.set("before", page.next_cursor);
          page = await client.get<ThreadPage>(`/api/threads?${params}`, { signal });
          items.push(...page.items);
        }
        if (active && started === generation && !signal.aborted) {
          for (const item of items) requestSequences.set(item, requestedSequence);
          for (const [id, local] of observed) {
            if (
              requestedSequence > local.sequence &&
              (!append || items.some((item) => item.id === id))
            ) {
              observed.delete(id);
            }
          }
          fetchedAt = Date.now();
          observedRevision = startedRevision;
          failedRequest = null;
          cursor = rebuild ? rangeEnd : page.next_cursor;
          hasAdditionalPages = append || (rebuild && items.length > 100);
          const nextCursor = cursor;
          setResult((current) => {
            const merged = new Map(
              [...(append && current?.key === key ? current.items : []), ...items].map((item) => [
                item.id,
                item,
              ]),
            );
            // 古い応答にない確定済みの行は、ローカルでの順序のまま先頭に保つ。
            const localOnly = (current?.items ?? []).filter(
              (item) =>
                !merged.has(item.id) && observed.get(item.id)?.thread.thread_archive_at === null,
            );
            for (const [id, local] of observed) {
              if (local.thread.thread_archive_at) merged.delete(id);
              else merged.set(id, local.thread);
            }
            return {
              items: [
                ...new Map(
                  [...localOnly, ...merged.values()].map((item) => [item.id, item]),
                ).values(),
              ],
              next_cursor: nextCursor,
              key,
              isLoading: false,
              isError: false,
            };
          });
        }
      } catch {
        // 一時的な取得失敗では直前の一覧を残し、再試行できるようにする。
        if (active && started === generation && !request.signal.aborted) {
          failedRequest = { append, cursor: before };
          setResult((current) =>
            current?.key === key ? { ...current, isLoading: false, isError: true } : current,
          );
        }
      } finally {
        if (active && request.signal.aborted) {
          setResult((current) => (current ? { ...current, isLoading: false } : current));
        }
        inFlight = false;
        if (active && pending) {
          const next = pending;
          pending = null;
          void refresh(next === "append");
        }
      }
    };
    const invalidate = () => {
      // 更新前に始まった取得結果で、更新後の一覧を上書きしない。
      fetchedAt = 0;
      generation += 1;
      void refresh();
    };
    const refreshFirst = () => {
      // focus・復帰・定期確認は、既に取得中ならその結果を共有する。
      if (!inFlight) void refresh();
      else if (controller?.signal.aborted) pending = "refresh";
    };
    loadMore = () => {
      const request = failedRequest;
      void refresh(request?.append ?? cursor !== null, request ? request.cursor : cursor);
    };
    if (
      !result ||
      result.isError ||
      observedRevision !== threadRevision() ||
      Date.now() - fetchedAt >= 60_000
    )
      void refresh();
    const interval = archived ? null : window.setInterval(refreshFirst, 60_000);
    window.addEventListener("focus", refreshFirst);
    window.addEventListener("online", refreshFirst);
    window.addEventListener(THREADS_CHANGED, invalidate);
    const visibility = () => {
      if (document.visibilityState === "hidden") controller?.abort();
      else refreshFirst();
    };
    document.addEventListener("visibilitychange", visibility);
    return () => {
      active = false;
      controller?.abort();
      if (result?.isLoading) {
        fetchedAt = 0;
        result = { ...result, isLoading: false };
      }
      loadMore = () => {};
      if (interval !== null) window.clearInterval(interval);
      window.removeEventListener("focus", refreshFirst);
      window.removeEventListener("online", refreshFirst);
      window.removeEventListener(THREADS_CHANGED, invalidate);
      document.removeEventListener("visibilitychange", visibility);
    };
  };
  return {
    observeThread: (thread: ThreadSummary) => {
      if (archived || (channelId && thread.channel_id !== channelId)) return;
      const previous = observed.get(thread.id)?.thread;
      if (previous && JSON.stringify(previous) === JSON.stringify(thread)) return;
      observed.set(thread.id, { thread, sequence: nextThreadIndexSequence() });
      setResult((current) => {
        const items = current?.items ?? [];
        const exists = items.some((item) => item.id === thread.id);
        return {
          ...(current ?? { ...EMPTY_PAGE, key, isLoading: false, isError: false }),
          items: thread.thread_archive_at
            ? items.filter((item) => item.id !== thread.id)
            : exists
              ? items.map((item) => (item.id === thread.id ? thread : item))
              : [thread, ...items],
        };
      });
    },
    snapshot: () => result,
    loadMore: () => loadMore(),
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      if (listeners.size === 1 && userId) stop = start();
      return () => {
        listeners.delete(listener);
        if (!listeners.size) {
          stop?.();
          stop = undefined;
        }
      };
    },
  };
}
const noSubscribe = () => () => {};
const noResult = () => null;

/** アクティブ一覧の60秒更新は表示中・購読中だけ。履歴は展開・操作時だけ取得する。 */
export function useThreadIndex(
  userId: string | null,
  {
    channelId,
    archived = false,
    enabled = true,
  }: { channelId?: string; archived?: boolean; enabled?: boolean } = {},
) {
  const { config } = useCollections();
  const key = JSON.stringify([userId, channelId, archived]);
  const store = useMemo(() => {
    let scoped = stores.get(config);
    if (!scoped) {
      scoped = new Map();
      stores.set(config, scoped);
    }
    let entry = scoped.get(key);
    if (!entry) {
      entry = createIndex(config, userId, channelId, archived, key);
      scoped.set(key, entry);
    }
    return entry;
  }, [config, key, userId, channelId, archived]);
  const current = useSyncExternalStore(
    enabled ? store.subscribe : noSubscribe,
    enabled ? store.snapshot : noResult,
    noResult,
  );
  return {
    threads: current?.items ?? EMPTY_PAGE.items,
    hasMore: !!current?.next_cursor,
    isLoading: current?.isLoading ?? false,
    isError: current?.isError ?? false,
    loadMore: store.loadMore,
    observeThread: store.observeThread,
  };
}
