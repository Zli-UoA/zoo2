import {
  type Collection,
  type InitialQueryBuilder,
  and,
  eq,
  isNull,
  not,
} from "@tanstack/react-db";

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

export interface ReplyCountSources {
  messages: Collection<MessageRow, string | number>;
}

/**
 * フィードの範囲。`parentId: null` はチャンネル直下（スレッドのルート）、
 * 文字列はそのメッセージへの返信を表す。
 */
export interface MessageFeedScope {
  parentId: string | null;
}

/**
 * メッセージと投稿者（users）を TanStack DB の live query で結合する。
 *
 * Electric の Shape は単一テーブル単位で、テーブル間 JOIN ができないため、
 * `messages` と `users` を別々に購読してクライアント側で結合する（非正規化しない）。
 * `deleted_at` が入った行はクエリ側で除外する。
 *
 * スレッド用に別 Shape / コレクションは作らず、同じチャンネルの messages を
 * `parent_id` で絞る（メイン一覧は `parent_id IS NULL`、スレッドは `= <root>`）。
 */
export function messageFeedQuery(
  q: InitialQueryBuilder,
  sources: MessageFeedSources,
  scope: MessageFeedScope = { parentId: null },
) {
  return q
    .from({ messages: sources.messages })
    .join({ users: sources.users }, ({ messages, users }) => eq(messages.user_id, users.id))
    .where(({ messages }) =>
      and(
        isNull(messages.deleted_at),
        scope.parentId === null
          ? isNull(messages.parent_id)
          : eq(messages.parent_id, scope.parentId),
      ),
    )
    .select(({ messages, users }) => ({
      id: messages.id,
      channel_id: messages.channel_id,
      user_id: messages.user_id,
      content: messages.content,
      card: messages.card,
      created_at: messages.created_at,
      parent_id: messages.parent_id,
      user_name: users.user_name,
      display_name: users.display_name,
    }))
    .orderBy(({ messages }) => messages.created_at);
}

/**
 * 返信数の集計用に、削除されていない返信の `parent_id` だけを購読する。
 * ルートごとの件数はクライアント側で数える（サーバの集計カラムは持たない）。
 */
export function replyParentQuery(q: InitialQueryBuilder, sources: ReplyCountSources) {
  return q
    .from({ messages: sources.messages })
    .where(({ messages }) => and(isNull(messages.deleted_at), not(isNull(messages.parent_id))))
    .select(({ messages }) => ({ parent_id: messages.parent_id }));
}
