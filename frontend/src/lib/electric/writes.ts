import type { ElectricCollectionUtils } from "@tanstack/electric-db-collection";
import type {
  DeleteMutationFnParams,
  InsertMutationFnParams,
  UpdateMutationFnParams,
} from "@tanstack/react-db";

import type { ApiClient } from "../api/client";
import type {
  AddChannelUserRequest,
  CreateChannelRequest,
  CreateMessageRequest,
  UpdateChannelRequest,
  UpdateMessageRequest,
} from "../api/types";
import { reactionKey } from "../emoji/kitchen";
import type { TxidMode } from "../config";
import { changeMessageMatcher, syncTxid } from "./txid";
import type {
  ChannelRow,
  ChannelUserRow,
  MessageReactionRow,
  MessageRow,
  NotificationRow,
} from "./types";

export interface WriteDependencies {
  client: ApiClient;
  txidMode: TxidMode;
  /**
   * 認証中のユーザー id を返す。`useCurrentUser` と同じストアを参照し、OIDC では
   * `/api/users/me` の解決後（dev では `VITE_DEV_USER_ID`）の値を返す。
   * `leave_channel` は認証ユーザー自身の行しか消せないため、channel_user の
   * 削除ハンドラで本人確認に使う（解決前は null なので拒否される）。
   */
  getCurrentUserId: () => string | null;
}

type MessageInsertParams = InsertMutationFnParams<
  MessageRow,
  string | number,
  ElectricCollectionUtils<MessageRow>
>;
type MessageUpdateParams = UpdateMutationFnParams<
  MessageRow,
  string | number,
  ElectricCollectionUtils<MessageRow>
>;
type MessageDeleteParams = DeleteMutationFnParams<
  MessageRow,
  string | number,
  ElectricCollectionUtils<MessageRow>
>;

type ChannelInsertParams = InsertMutationFnParams<
  ChannelRow,
  string | number,
  ElectricCollectionUtils<ChannelRow>
>;
type ChannelUpdateParams = UpdateMutationFnParams<
  ChannelRow,
  string | number,
  ElectricCollectionUtils<ChannelRow>
>;

type ChannelUserInsertParams = InsertMutationFnParams<
  ChannelUserRow,
  string | number,
  ElectricCollectionUtils<ChannelUserRow>
>;
type ChannelUserDeleteParams = DeleteMutationFnParams<
  ChannelUserRow,
  string | number,
  ElectricCollectionUtils<ChannelUserRow>
>;

/**
 * channel_user の書き込み種別。join と add はどちらも INSERT だが、叩く
 * エンドポイントが違う（`/join` は認証ユーザー自身、`/users` は指定ユーザー）。
 * `collection.insert()` の metadata に載せて使い分ける。
 */
export type ChannelUserWriteMetadata =
  | { action: "join" }
  | { action: "add"; user_id: string }
  | { action: "leave" };

function readChannelUserMetadata(metadata: unknown): ChannelUserWriteMetadata {
  if (metadata && typeof metadata === "object" && "action" in metadata) {
    return metadata as ChannelUserWriteMetadata;
  }
  return { action: "join" };
}

export interface MessageWriteMetadata {
  attachment_ids?: string[];
}

function readMessageMetadata(metadata: unknown): MessageWriteMetadata {
  if (!metadata || typeof metadata !== "object" || !("attachment_ids" in metadata)) {
    return {};
  }
  const value = (metadata as { attachment_ids?: unknown }).attachment_ids;
  if (!Array.isArray(value)) {
    return {};
  }
  return {
    attachment_ids: value.filter((id): id is string => typeof id === "string"),
  };
}

/**
 * メッセージの送信・編集・論理削除。
 *
 * 論理削除は「`deleted_at` を更新する」楽観更新として扱い、REST は
 * `DELETE /api/messages/{id}` を叩く。サーバ側も `deleted_at` の UPDATE なので、
 * Electric が流す operation は `delete` ではなく `update` になる。楽観更新も
 * `update` に揃えることで、delete→insert の再出現（ちらつき）を避ける。
 */
export function createMessageWrites({ client, txidMode }: WriteDependencies) {
  async function onInsert({ transaction, collection }: MessageInsertParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      const body: CreateMessageRequest = {
        id: row.id,
        channel_id: row.channel_id,
        content: row.content,
      };
      // 通常メッセージではキー自体を送らない（バックエンドの既定はルート）。
      if (row.parent_id) {
        body.parent_id = row.parent_id;
      }
      const attachmentIds = readMessageMetadata(mutation.metadata).attachment_ids;
      if (attachmentIds?.length) {
        body.attachment_ids = attachmentIds;
      }
      const { txid } = await client.post<MessageRow>("/api/messages", body);
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<MessageRow>("insert", (value) => value.id === row.id),
      });
    }
  }

  async function onUpdate({ transaction, collection }: MessageUpdateParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      const changes = mutation.changes as Partial<MessageRow>;
      const isSoftDelete = "deleted_at" in changes && changes.deleted_at != null;

      if (isSoftDelete) {
        const { txid } = await client.delete<MessageRow>(`/api/messages/${row.id}`);
        await syncTxid({
          mode: txidMode,
          txid,
          target: collection.utils,
          match: changeMessageMatcher<MessageRow>(
            "update",
            (value) => value.id === row.id && value.deleted_at != null,
          ),
        });
        continue;
      }

      const body: UpdateMessageRequest = { content: row.content };
      const { txid } = await client.patch<MessageRow>(`/api/messages/${row.id}`, body);
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<MessageRow>(
          "update",
          (value) => value.id === row.id && value.content === row.content,
        ),
      });
    }
  }

  async function onDelete({ transaction, collection }: MessageDeleteParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.original;
      const { txid } = await client.delete<MessageRow>(`/api/messages/${row.id}`);
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<MessageRow>(
          "update",
          (value) => value.id === row.id && value.deleted_at != null,
        ),
      });
    }
  }

  return { onInsert, onUpdate, onDelete };
}

/** チャンネルの作成・更新。作成時はサーバ側で作成者の channel_user も追加される。 */
export function createChannelWrites({ client, txidMode }: WriteDependencies) {
  async function onInsert({ transaction, collection }: ChannelInsertParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      const body: CreateChannelRequest = {
        id: row.id,
        channel_name: row.channel_name,
        description: row.description,
        is_private: row.is_private,
      };
      const { txid } = await client.post<ChannelRow>("/api/channels", body);
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<ChannelRow>("insert", (value) => value.id === row.id),
      });
    }
  }

  async function onUpdate({ transaction, collection }: ChannelUpdateParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      const changes = mutation.changes as Partial<ChannelRow>;
      const body: UpdateChannelRequest = {};
      if ("channel_name" in changes) {
        body.channel_name = changes.channel_name;
      }
      if ("description" in changes) {
        body.description = changes.description ?? null;
      }
      if ("is_private" in changes) {
        body.is_private = changes.is_private;
      }

      const { txid } = await client.patch<ChannelRow>(`/api/channels/${row.id}`, body);
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<ChannelRow>("update", (value) => value.id === row.id),
      });
    }
  }

  return { onInsert, onUpdate };
}

/**
 * channel_user の参加・退出・メンバー追加。
 *
 * 参加/追加は INSERT、退出は DELETE として扱い、いずれも channel_user
 * コレクションで `txid` を待つ。既定の proxy では channels Shape をバックエンドが
 * メンバーシップで絞るため、参加/退出に応じて channels 側の行が move-in / move-out
 * する（同じトランザクションの `txid` で配信される）。ここで待つのは channel_user
 * 側の `txid` で、チャンネル一覧は続けて更新される。
 *
 * なお join / add は同じ行が既にあるとサーバは既存行を返すだけで Electric に変更が
 * 流れず、`awaitTxId` がタイムアウトする。呼び出し側で重複追加を避けること。
 *
 * 退出も同様に制約がある。`POST /api/channels/{id}/leave` は「認証ユーザー自身」の
 * 行を消すエンドポイントで、他メンバーの行を指定して消す手段はない。したがって
 * 現在のユーザー以外の行を削除しようとしたら throw して楽観状態を戻す（他人を
 * 退出させたように見せかけつつ実際は自分が抜ける、という取り違えを防ぐ）。
 * メンバー削除 API ができたらここを差し替える。
 */
export function createChannelUserWrites({ client, txidMode, getCurrentUserId }: WriteDependencies) {
  async function onInsert({ transaction, collection }: ChannelUserInsertParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      const metadata = readChannelUserMetadata(mutation.metadata);
      const action = metadata.action === "add" ? "add" : "join";

      if (action === "add") {
        const body: AddChannelUserRequest = { user_id: row.user_id };
        const { txid } = await client.post<ChannelUserRow>(
          `/api/channels/${row.channel_id}/users`,
          body,
        );
        await waitForChannelUserSync("insert", row, collection.utils, txid, txidMode);
        continue;
      }

      const { txid } = await client.post<ChannelUserRow>(`/api/channels/${row.channel_id}/join`);
      await waitForChannelUserSync("insert", row, collection.utils, txid, txidMode);
    }
  }

  async function onDelete({ transaction, collection }: ChannelUserDeleteParams): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.original;
      const currentUserId = getCurrentUserId();
      if (row.user_id !== currentUserId) {
        throw new Error(
          `/leave only removes the current user's membership; refusing to leave for user ${row.user_id}`,
        );
      }
      const { txid } = await client.post<ChannelUserRow>(`/api/channels/${row.channel_id}/leave`);
      await waitForChannelUserSync("delete", row, collection.utils, txid, txidMode);
    }
  }

  return { onInsert, onDelete };
}

async function waitForChannelUserSync(
  operation: "insert" | "delete",
  row: ChannelUserRow,
  target: ElectricCollectionUtils<ChannelUserRow>,
  txid: string | null,
  txidMode: TxidMode,
): Promise<void> {
  await syncTxid({
    mode: txidMode,
    txid,
    target,
    match: changeMessageMatcher<ChannelUserRow>(
      operation,
      (value) => value.user_id === row.user_id && value.channel_id === row.channel_id,
    ),
  });
}

/** リアクションの追加・解除。REST の正規化後の値で同期を照合する。 */
export function createReactionWrites({ client, txidMode, getCurrentUserId }: WriteDependencies) {
  async function onInsert({
    transaction,
    collection,
  }: InsertMutationFnParams<
    MessageReactionRow,
    string | number,
    ElectricCollectionUtils<MessageReactionRow>
  >): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.modified;
      if (!getCurrentUserId() || row.user_id !== getCurrentUserId())
        throw new Error("自分のリアクションだけ追加できます");
      const { data, txid } = await client.post<MessageReactionRow>(
        `/api/messages/${row.message_id}/reactions`,
        { id: row.id, emoji: row.emoji },
      );
      // A concurrent tab may have added this reaction already. In that case the
      // API returns the existing row without a new INSERT or replicated txid.
      if (data.id !== row.id) continue;
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<MessageReactionRow>("insert", (value) => value.id === data.id),
      });
    }
  }
  async function onDelete({
    transaction,
    collection,
  }: DeleteMutationFnParams<
    MessageReactionRow,
    string | number,
    ElectricCollectionUtils<MessageReactionRow>
  >): Promise<void> {
    for (const mutation of transaction.mutations) {
      const row = mutation.original;
      if (!getCurrentUserId() || row.user_id !== getCurrentUserId())
        throw new Error("自分のリアクションだけ解除できます");
      const { txid } = await client.delete<MessageReactionRow>(
        `/api/messages/${row.message_id}/reactions?${new URLSearchParams({ emoji: row.emoji })}`,
      );
      await syncTxid({
        mode: txidMode,
        txid,
        target: collection.utils,
        match: changeMessageMatcher<MessageReactionRow>(
          "delete",
          (value) =>
            value.id === row.id ||
            (value.message_id === row.message_id &&
              value.user_id === row.user_id &&
              reactionKey(value.emoji) === reactionKey(row.emoji)),
        ),
      });
    }
  }
  return { onInsert, onDelete };
}

/** 通知の既読化。まとめて更新する場合は read-all API を使う。 */
export function createNotificationWrites({
  client,
  txidMode,
  getCurrentUserId,
}: WriteDependencies) {
  async function onUpdate({
    transaction,
    collection,
  }: UpdateMutationFnParams<
    NotificationRow,
    string | number,
    ElectricCollectionUtils<NotificationRow>
  >): Promise<void> {
    const mutations = transaction.mutations;
    for (const mutation of mutations) {
      if (
        !getCurrentUserId() ||
        mutation.modified.user_id !== getCurrentUserId() ||
        !mutation.modified.is_read
      )
        throw new Error("Only your own notifications can be marked read");
    }
    if (mutations.length === 0) return;
    const { txid } =
      mutations.length > 1
        ? await client.post<{ updated_count: number }>("/api/notifications/read-all")
        : await client.patch<NotificationRow>(
            `/api/notifications/${mutations[0]!.modified.id}/read`,
          );
    const ids = new Set(mutations.map((mutation) => mutation.modified.id));
    await syncTxid({
      mode: txidMode,
      txid,
      target: collection.utils,
      match: changeMessageMatcher<NotificationRow>(
        "update",
        (row) => ids.has(row.id) && row.is_read,
      ),
    });
  }
  return { onUpdate };
}
