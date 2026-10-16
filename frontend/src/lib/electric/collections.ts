import { BasicIndex, createCollection } from "@tanstack/react-db";
import { electricCollectionOptions } from "@tanstack/electric-db-collection";

import { resolveConfig, type AppConfig } from "../config";
import { createShapeFetch, type ShapeFetchSource } from "./headers";
import { buildShapeRequestUrl, type ShapeQueryOptions } from "./query";
import type { ShapeName } from "./schema";
import type { ChannelRow, ChannelUserRow, MessageRow, UserRow } from "./types";

/**
 * Shape 購読に付与する認証情報を決める。
 *
 * direct（Electric 直結）は認可なしの開発専用経路で、Electric の CORS は
 * `authorization` / `X-Dev-User` を許可していないため、認証ヘッダを一切付けない
 * （付けると preflight で全リクエストが失敗する）。proxy はバックエンドが REST と
 * 同じヘッダを受け付けるため、`X-Dev-User` をそのまま渡す。
 */
export function shapeAuthSource(config: AppConfig): ShapeFetchSource {
  if (config.shapeMode === "proxy") {
    return { devUserId: config.devUserId };
  }
  return { auth: false };
}

function shapeOptions(shape: ShapeName, config: AppConfig, options: ShapeQueryOptions = {}) {
  return {
    url: buildShapeRequestUrl(shape, config, options),
    fetchClient: createShapeFetch(shapeAuthSource(config)),
  };
}

function channelsCollection(config: AppConfig) {
  const collection = createCollection(
    electricCollectionOptions<ChannelRow>({
      id: "channels",
      getKey: (row) => row.id,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("channels", config),
    }),
  );
  collection.createIndex((row) => row.id);
  return collection;
}

function usersCollection(config: AppConfig) {
  const collection = createCollection(
    electricCollectionOptions<UserRow>({
      id: "users",
      getKey: (row) => row.id,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("users", config),
    }),
  );
  // messages ⋈ users の結合キー。live query の警告を避けるため索引を張る。
  collection.createIndex((row) => row.id);
  return collection;
}

/**
 * メンバー一覧用の channel_user。今は `channel_id` を指定せず全件購読し、
 * `useChannelMembers` がチャンネル単位で絞り込む。`shapeQueryParams` の
 * `channelUsers` + `channelId` 分岐は、将来チャンネル単位で購読する場合用。
 *
 * proxy で `channel_id` なしの場合、バックエンドは channel_user のサブクエリ経路を
 * 使うため、proxy に切り替えるときに別途確認する（docs/electric-sync-requirements.md §1）。
 */
function channelUsersCollection(config: AppConfig) {
  const collection = createCollection(
    electricCollectionOptions<ChannelUserRow>({
      id: "channel_user",
      getKey: (row) => `${row.user_id}:${row.channel_id}`,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("channelUsers", config),
    }),
  );
  // channel_id 単位に絞り込む `useChannelMembers` の述語に合わせて索引を張る。
  collection.createIndex((row) => row.channel_id);
  return collection;
}

function messagesCollection(channelId: string, config: AppConfig) {
  const collection = createCollection(
    electricCollectionOptions<MessageRow>({
      id: `messages:${channelId}`,
      getKey: (row) => row.id,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("messages", config, { channelId }),
    }),
  );
  collection.createIndex((row) => row.id);
  return collection;
}

export type ChannelsCollection = ReturnType<typeof channelsCollection>;
export type UsersCollection = ReturnType<typeof usersCollection>;
export type ChannelUsersCollection = ReturnType<typeof channelUsersCollection>;
export type MessagesCollection = ReturnType<typeof messagesCollection>;

export interface CollectionsBundle {
  config: AppConfig;
  channels: ChannelsCollection;
  users: UsersCollection;
  channelUsers: ChannelUsersCollection;
  /** `channel_id` 単位で生成しキャッシュした messages コレクションを返す。 */
  messages: (channelId: string) => MessagesCollection;
}

export function isBrowser(): boolean {
  return typeof window !== "undefined";
}

let resolvedConfig: AppConfig | null = null;
let bundle: CollectionsBundle | null = null;

function defaultConfig(): AppConfig {
  resolvedConfig ??= resolveConfig();
  return resolvedConfig;
}

/**
 * ブラウザ専用のコレクションバンドルを返す（SSR では呼ばない）。
 *
 * Electric の ShapeStream はブラウザ API に依存するため、モジュール直下ではなく
 * client コンポーネント / フックから呼び出す。messages だけは channel_id 単位で
 * 遅延生成してキャッシュする。
 */
export function getCollections(config: AppConfig = defaultConfig()): CollectionsBundle {
  if (!isBrowser()) {
    throw new Error("Electric collections can only be created in the browser");
  }
  if (bundle && bundle.config === config) {
    return bundle;
  }

  const messageCache = new Map<string, MessagesCollection>();
  bundle = {
    config,
    channels: channelsCollection(config),
    users: usersCollection(config),
    channelUsers: channelUsersCollection(config),
    messages: (channelId) => {
      let collection = messageCache.get(channelId);
      if (!collection) {
        collection = messagesCollection(channelId, config);
        messageCache.set(channelId, collection);
      }
      return collection;
    },
  };
  return bundle;
}
