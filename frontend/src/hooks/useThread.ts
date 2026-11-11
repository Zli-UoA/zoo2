import { useLiveInfiniteQuery, useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";

import { useCollections } from "../lib/electric/collections-context";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import { MESSAGE_PAGE_SIZE } from "../lib/electric/messageOrder";
import { useOptimisticDisplay } from "./useOptimisticDisplay";

/**
 * スレッドが開いていないときの、何にも一致しない親の ID (空の範囲を要求するだけで、取得は 1 回で済む)。
 * parent_id は text なので、NUL を含まない固定の文字列にする (PostgreSQL の text は NUL を受け付けない)。
 */
const NO_THREAD = "no-thread:none";
const NO_ROWS: never[] = [];

/**
 * 指定ルートへの返信を、メイン一覧と同じ messages コレクションの live query で購読する。
 * スレッド専用の Shape / collection は作らない。メイン一覧と同じく、新しい方から
 * `MESSAGE_PAGE_SIZE` 件ずつ (`loadOlder()` で古い方へ)。`data` は古い順。
 */
export function useThreadFeed(channelId: string, rootId: string | null) {
  const { messages, users } = useCollections();
  const query = useLiveInfiniteQuery(
    (q) =>
      messageFeedQuery(
        q,
        { messages: messages(channelId), users },
        { parentId: rootId ?? NO_THREAD },
      ),
    { pageSize: MESSAGE_PAGE_SIZE },
  );
  const oldestFirst = useMemo(() => [...(query.data ?? [])].reverse(), [query.data]);
  const data = useOptimisticDisplay(channelId, rootId ? oldestFirst : NO_ROWS);
  return {
    ...query,
    data,
    loadOlder: query.fetchNextPage,
    hasOlder: query.hasNextPage,
    isLoadingOlder: query.isFetchingNextPage,
  };
}

/**
 * ID で 1 件のメッセージを取る (スレッドの親、通知やリンクの遷移先)。メイン一覧の範囲とは関係なく、
 * Electric の subset snapshot で、その行だけを取る。削除済みの行も返す (`deleted_at`)。
 */
export function useMessageById(channelId: string | null, messageId: string | null) {
  const { messages, users } = useCollections();
  const query = useLiveQuery((q) =>
    channelId && messageId
      ? messageFeedQuery(
          q,
          { messages: messages(channelId), users },
          { parentId: null, messageId, includeDeleted: true },
        )
      : undefined,
  );
  const data = useOptimisticDisplay(channelId, query.data);
  return { ...query, data };
}

/** スレッドの親を、メイン一覧の範囲とは関係なく取る。 */
export function useThreadRoot(channelId: string | null, rootId: string | null) {
  return useMessageById(channelId, rootId);
}
