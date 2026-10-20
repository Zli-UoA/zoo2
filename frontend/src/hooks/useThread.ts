import { useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";

import { getCollections } from "../lib/electric/collections";
import { messageFeedQuery, replyParentQuery } from "../lib/electric/messageFeed";
import { countRepliesByParent } from "../lib/electric/threads";

/**
 * 指定ルートへの返信を、メイン一覧と同じ messages コレクションの live query で
 * 購読する。スレッド専用の Shape / collection は作らない。
 */
export function useThreadFeed(channelId: string | null, rootId: string | null) {
  const { messages, users } = getCollections();
  return useLiveQuery((q) =>
    channelId && rootId
      ? messageFeedQuery(q, { messages: messages(channelId), users }, { parentId: rootId })
      : undefined,
  );
}

/**
 * チャンネル内の返信を `parent_id` ごとに数える。`reply_count` カラムは
 * 持たないため、削除されていない返信の行からクライアントで集計する。
 */
export function useReplyCounts(channelId: string | null): Map<string, number> {
  const { messages } = getCollections();
  const query = useLiveQuery((q) =>
    channelId ? replyParentQuery(q, { messages: messages(channelId) }) : undefined,
  );
  return useMemo(() => countRepliesByParent(query.data ?? []), [query.data]);
}
