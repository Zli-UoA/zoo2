import { type Collection, type InitialQueryBuilder, eq, isNull } from "@tanstack/react-db";

import type { MessageRow, UserRow } from "./types";

/**
 * `createCollection` と Electric コレクションが返す公開型 `Collection` に合わせる。
 * `CollectionImpl`（実装クラス）は `tsc` では構造的に一致するが、型チェッカ間で
 * 解決が揺れて誤検知の原因になるため、公開 API の型注釈には使わない。
 */
export interface MessageFeedSources {
  messages: Collection<MessageRow, string | number>;
  users: Collection<UserRow, string | number>;
}

/**
 * メッセージと投稿者（users）を TanStack DB の live query で結合する。
 *
 * Electric の Shape は単一テーブル単位で、テーブル間 JOIN ができないため、
 * `messages` と `users` を別々に購読してクライアント側で結合する（非正規化しない）。
 * `deleted_at` が入った行はクエリ側で除外する。
 */
export function messageFeedQuery(q: InitialQueryBuilder, sources: MessageFeedSources) {
  return q
    .from({ messages: sources.messages })
    .join({ users: sources.users }, ({ messages, users }) => eq(messages.user_id, users.id))
    .where(({ messages }) => isNull(messages.deleted_at))
    .select(({ messages, users }) => ({
      id: messages.id,
      channel_id: messages.channel_id,
      user_id: messages.user_id,
      content: messages.content,
      created_at: messages.created_at,
      user_name: users.user_name,
      display_name: users.display_name,
    }))
    .orderBy(({ messages }) => messages.created_at);
}
