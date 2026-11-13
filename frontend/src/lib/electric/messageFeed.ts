import {
  type Collection,
  type InitialQueryBuilder,
  and,
  caseWhen,
  eq,
  isNull,
} from "@tanstack/react-db";

import { MESSAGE_ORDER } from "./messageOrder";
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
 * フィードの範囲。`parentId: null` はチャンネル直下（スレッドのルート）、
 * 文字列はそのメッセージへの返信を表す。範囲 (何件読むか) は、クエリの `limit` を呼び出し側が
 * `useLiveInfiniteQuery` で動かして決める (端末に同期するのは、その範囲だけ)。
 */
export interface MessageFeedScope {
  parentId: string | null;
  messageId?: string;
  /** 削除済みの行も返す (スレッドの親が削除されたかを調べる)。 */
  includeDeleted?: boolean;
}

/**
 * メッセージと投稿者（users）を TanStack DB の live query で結合する。
 *
 * 表示用VIEWの投稿者を優先し、旧キャッシュや楽観更新だけ users で補完する。
 * 添付・絵文字・リアクション・返信数も display_data として同時に届く。
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
  return (
    q
      .from({ messages: sources.messages })
      .join({ users: sources.users }, ({ messages, users }) => eq(messages.user_id, users.id))
      .where(({ messages }) => {
        const parent = scope.messageId
          ? eq(messages.id, scope.messageId)
          : scope.parentId === null
            ? isNull(messages.parent_id)
            : eq(messages.parent_id, scope.parentId);
        return scope.includeDeleted ? parent : and(isNull(messages.deleted_at), parent);
      })
      .select(({ messages, users }) => ({
        id: messages.id,
        display_data: messages.display_data,
        channel_id: messages.channel_id,
        user_id: messages.user_id,
        content: messages.content,
        card: messages.card,
        app_blocks: messages.app_blocks,
        app_attachments: messages.app_attachments,
        app_unfurls: messages.app_unfurls,
        app_origin_id: messages.app_origin_id,
        home_post_id: messages.home_post_id,
        apub: messages.apub,
        apub_visibility: messages.apub_visibility,
        is_bot: caseWhen(
          eq(messages.display_data?.author?.id, messages.user_id),
          messages.display_data?.author?.is_bot,
          users.is_bot,
        ),
        app_id: caseWhen(
          eq(messages.display_data?.author?.id, messages.user_id),
          messages.display_data?.author?.app_id,
          users.app_id,
        ),
        avatar_url: caseWhen(
          eq(messages.display_data?.author?.id, messages.user_id),
          messages.display_data?.author?.avatar_url,
          users.avatar_url,
        ),
        created_at: messages.created_at,
        updated_at: messages.updated_at,
        deleted_at: messages.deleted_at,
        parent_id: messages.parent_id,
        reply_to_message_id: messages.reply_to_message_id,
        thread_archive_at: messages.thread_archive_at,
        user_name: caseWhen(
          eq(messages.display_data?.author?.id, messages.user_id),
          messages.display_data?.author?.user_name,
          users.user_name,
        ),
        display_name: caseWhen(
          eq(messages.display_data?.author?.id, messages.user_id),
          messages.display_data?.author?.display_name,
          users.display_name,
        ),
      }))
      // 新しい順。`limit` を付けると、必要な範囲だけを Electric の subset snapshot で取る
      // (`lexical` でないと、全件を取るフォールバックになる。messageOrder.ts)。表示は、呼び出し側で反転する。
      .orderBy(({ messages }) => messages.created_at, MESSAGE_ORDER)
  );
}
