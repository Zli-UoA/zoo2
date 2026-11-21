import { useMemo, useSyncExternalStore } from "react";

import {
  appendSearchItems,
  searchRequestPath,
  type SearchItem,
  type SearchKind,
  type SearchPage,
} from "../components/chat/search/globalSearch";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

export interface GlobalSearchState {
  items: SearchItem[];
  nextCursor: string | null;
  searchedUntil: string | null;
  /** 最初のページを受け取ったか。 */
  loaded: boolean;
  isLoading: boolean;
  isError: boolean;
}

const IDLE: GlobalSearchState = {
  items: [],
  nextCursor: null,
  searchedUntil: null,
  loaded: false,
  isLoading: false,
  isError: false,
};

/** 1 回のリクエストで最大 12 か月を探すため、通常の読み取りより長く待つ。 */
const REQUEST_TIMEOUT_MS = 20_000;
/** 結果を開いて戻ったときは、この時間内なら取り直さない。 */
const REUSE_MS = 5 * 60_000;
/** 設定・利用者ごとに残す検索語の数。 */
const MAX_ENTRIES = 8;

type Config = Parameters<typeof createConfiguredApiClient>[0];

/**
 * 1 つの検索語・種類の結果。ページは直列に取得し、取得中の追加要求は無視する。
 * 最後の購読が外れたら進行中の取得を中断する（StrictMode の再購読では中断しない）。
 */
function createSearchEntry(config: Config, query: string, kind: SearchKind) {
  const client = createConfiguredApiClient(config);
  let state: GlobalSearchState = IDLE;
  let fetchedAt = 0;
  let generation = 0;
  let controller: AbortController | null = null;
  /** 失敗した取得のカーソル（1 ページ目なら null）。失敗していなければ undefined。 */
  let failedCursor: string | null | undefined;
  let pendingStop: ReturnType<typeof setTimeout> | null = null;
  const listeners = new Set<() => void>();
  const set = (next: GlobalSearchState) => {
    state = next;
    listeners.forEach((listener) => listener());
  };
  const abort = () => {
    if (!controller) return;
    generation += 1;
    controller.abort();
    controller = null;
    state = { ...state, isLoading: false };
  };
  const fetchPage = (cursor: string | null) => {
    if (controller) return;
    const started = ++generation;
    const request = new AbortController();
    controller = request;
    set({ ...state, isLoading: true, isError: false });
    void client
      .get<SearchPage>(searchRequestPath(query, kind, cursor), {
        signal: AbortSignal.any([request.signal, AbortSignal.timeout(REQUEST_TIMEOUT_MS)]),
      })
      .then(
        (page) => {
          if (started !== generation) return;
          controller = null;
          failedCursor = undefined;
          fetchedAt = Date.now();
          set({
            items: cursor ? appendSearchItems(state.items, page.items) : page.items,
            nextCursor: page.next_cursor,
            searchedUntil: page.searched_until,
            loaded: true,
            isLoading: false,
            isError: false,
          });
        },
        () => {
          if (started !== generation) return;
          controller = null;
          // 取得済みの結果は残し、失敗した取得（1 ページ目の取り直しも含む）を同じ位置から再試行できるようにする。
          failedCursor = cursor;
          set({ ...state, isLoading: false, isError: true });
        },
      );
  };
  return {
    snapshot: () => state,
    /**
     * 続き（さらに古い月）を読む。失敗した取得があれば、同じカーソルでやり直す（1 ページ目の取り直しが
     * 失敗していれば 1 ページ目から）。
     */
    loadMore: () => {
      if (!listeners.size) return;
      if (failedCursor !== undefined) fetchPage(failedCursor);
      else if (!state.loaded) fetchPage(null);
      else if (state.nextCursor) fetchPage(state.nextCursor);
    },
    /** 同じ語で検索し直す。進行中の取得は捨てる。 */
    refresh: () => {
      if (!listeners.size) return;
      abort();
      fetchPage(null);
    },
    get idle() {
      return listeners.size === 0;
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      if (pendingStop) {
        clearTimeout(pendingStop);
        pendingStop = null;
      }
      if (
        listeners.size === 1 &&
        !controller &&
        (!state.loaded || Date.now() - fetchedAt >= REUSE_MS)
      )
        fetchPage(null);
      return () => {
        listeners.delete(listener);
        if (listeners.size || pendingStop) return;
        pendingStop = setTimeout(() => {
          pendingStop = null;
          if (!listeners.size) abort();
        }, 0);
      };
    },
  };
}

type SearchEntry = ReturnType<typeof createSearchEntry>;
const stores = new WeakMap<object, Map<string, SearchEntry>>();

function entryFor(config: Config, key: string, query: string, kind: SearchKind) {
  let scoped = stores.get(config);
  if (!scoped) {
    scoped = new Map();
    stores.set(config, scoped);
  }
  let entry = scoped.get(key);
  if (entry) {
    // 最近使ったものを後ろへ。
    scoped.delete(key);
  } else {
    entry = createSearchEntry(config, query, kind);
  }
  scoped.set(key, entry);
  for (const [oldKey, old] of scoped) {
    if (scoped.size <= MAX_ENTRIES) break;
    if (old.idle && oldKey !== key) scoped.delete(oldKey);
  }
  return entry;
}

const noSubscribe = () => () => {};
const idleSnapshot = () => IDLE;
const noop = () => {};

/**
 * `GET /api/search` を読む。語・種類・アカウントごとに結果を持ち、画面を離れて戻ったときは
 * 5 分以内なら取り直さない。続きは利用者の操作（loadMore）でだけ読む。
 */
export function useGlobalSearch(query: string, kind: SearchKind) {
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const enabled = !!currentUserId && query.length > 0;
  const key = JSON.stringify([currentUserId, kind, query]);
  const entry = useMemo(
    () => (enabled ? entryFor(config, key, query, kind) : null),
    [enabled, config, key, query, kind],
  );
  const state = useSyncExternalStore(
    entry?.subscribe ?? noSubscribe,
    entry?.snapshot ?? idleSnapshot,
    idleSnapshot,
  );
  return {
    ...state,
    loadMore: entry?.loadMore ?? noop,
    refresh: entry?.refresh ?? noop,
  };
}
