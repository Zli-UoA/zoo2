import { eq, isNull, useLiveQuery } from "@tanstack/react-db";

import { getCollections } from "../lib/electric/collections";
import { messageFeedQuery } from "../lib/electric/messageFeed";

/** 投稿者名の解決に使う全ユーザーを購読する。 */
export function useUsers() {
  const { users } = getCollections();
  return useLiveQuery((q) => q.from({ users }));
}

/** 論理削除されていないチャンネル一覧を購読する。 */
export function useActiveChannels() {
  const { channels } = getCollections();
  return useLiveQuery((q) =>
    q
      .from({ channels })
      .where(({ channels }) => isNull(channels.deleted_at))
      .orderBy(({ channels }) => channels.created_at),
  );
}

/**
 * 指定チャンネルのメッセージを投稿者名（users）と結合して購読する。
 * `channelId` が無いときは購読しない。
 */
export function useMessageFeed(channelId: string | null) {
  const { messages, users } = getCollections();
  return useLiveQuery((q) =>
    channelId ? messageFeedQuery(q, { messages: messages(channelId), users }) : undefined,
  );
}

/** 指定チャンネルの参加メンバー（channel_user）を購読する。 */
export function useChannelMembers(channelId: string | null) {
  const { channelUsers } = getCollections();
  return useLiveQuery((q) =>
    channelId
      ? q.from({ channelUsers }).where(({ channelUsers }) => eq(channelUsers.channel_id, channelId))
      : undefined,
  );
}

/** チャンネル内のメッセージへのリアクションを購読する。 */
export function useMessageReactions(channelId: string) {
  const { reactions } = getCollections();
  return useLiveQuery((q) => q.from({ reactions: reactions(channelId) }));
}
