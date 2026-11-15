import {
  createCollection,
  localOnlyCollectionOptions,
  useLiveInfiniteQuery,
  useLiveQuery,
} from "@tanstack/react-db";
import { useMemo } from "react";

import { useCollections } from "../lib/electric/collections-context";
import { messageFeedQuery } from "../lib/electric/messageFeed";
import { MESSAGE_PAGE_SIZE } from "../lib/electric/messageOrder";
import type { MessageRow } from "../lib/electric/types";
import { useOptimisticDisplay } from "./useOptimisticDisplay";

const NO_ROWS: never[] = [];

/**
 * 指定ルートへの返信を、メイン一覧と同じ messages コレクションの live query で購読する。
 * スレッド専用の Shape / collection は作らない。メイン一覧と同じく、新しい方から
 * `MESSAGE_PAGE_SIZE` 件ずつ (`loadOlder()` で古い方へ)。`data` は古い順。
 */
export function useThreadFeed(channelId: string, rootId: string | null) {
  const { messages, users } = useCollections();
  const empty = useMemo(
    () =>
      createCollection(
        localOnlyCollectionOptions<MessageRow>({
          getKey: (row) => row.id,
          initialData: [],
        }),
      ),
    [],
  );
  const query = useLiveInfiniteQuery(
    (q) =>
      messageFeedQuery(
        q,
        { messages: rootId ? messages(channelId) : empty, users },
        { parentId: rootId },
      ),
    { pageSize: MESSAGE_PAGE_SIZE },
  );
  const oldestFirst = useMemo(() => [...(query.data ?? [])].reverse(), [query.data]);
  const data = useOptimisticDisplay(rootId ? channelId : null, rootId ? oldestFirst : NO_ROWS);
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
  const data = useOptimisticDisplay(channelId && messageId ? channelId : null, query.data);
  return { ...query, data };
}

/** スレッドの親を、メイン一覧の範囲とは関係なく取る。 */
export function useThreadRoot(channelId: string | null, rootId: string | null) {
  return useMessageById(channelId, rootId);
}
