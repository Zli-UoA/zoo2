import { useLiveQuery } from "@tanstack/react-db";
import { useMemo } from "react";

import { useCollections } from "../lib/electric/collections-context";
import { buildLeaveDateMap } from "../components/chat/channelSearch";

/**
 * 自分が退出したチャンネルの退出日（channel_id → 退出日）。参加中のチャンネルは含めない。
 * 退出日での並び替え（`compareByLeaveDate`）に使う。
 */
export function useChannelLeaveDates(
  currentUserId: string | null | undefined,
  joinedChannelIds: ReadonlySet<string>,
): ReadonlyMap<string, string> {
  const { channelLeaves } = useCollections();
  // ユーザーが未確定の間は、空のコレクションとして扱う（購読しない）。
  const collection = useMemo(
    () => (currentUserId ? channelLeaves(currentUserId) : null),
    [channelLeaves, currentUserId],
  );
  const query = useLiveQuery(
    (q) => (collection ? q.from({ leave: collection }) : undefined),
    [collection],
  );
  return useMemo(
    () => buildLeaveDateMap(query.data ?? [], joinedChannelIds),
    [query.data, joinedChannelIds],
  );
}
