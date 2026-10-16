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
import type { TxidMode } from "../config";
import { changeMessageMatcher, syncTxid } from "./txid";
import type { ChannelRow, ChannelUserRow, MessageRow } from "./types";

export interface WriteDependencies {
  client: ApiClient;
  txidMode: TxidMode;
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
 * コレクションで `txid` を待つ。direct モードの channels Shape は全チャンネルを
 * 配信しメンバーシップで絞り込まないため、参加/退出で channels 側の行は動かない。
 * 将来 proxy で channels をメンバーシップで絞る（join/leave が move-in/out になる）
 * 構成に切り替える場合は、channels コレクション側でも待つ必要がある。
 *
 * なお join / add は同じ行が既にあるとサーバは既存行を返すだけで Electric に変更が
 * 流れず、`awaitTxId` がタイムアウトする。呼び出し側で重複追加を避けること。
 */
export function createChannelUserWrites({ client, txidMode }: WriteDependencies) {
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
  txid: string,
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
