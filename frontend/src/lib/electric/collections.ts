import { BasicIndex, createCollection } from "@tanstack/react-db";
import { electricCollectionOptions } from "@tanstack/electric-db-collection";

import { resolveConfig, type AppConfig } from "../config";
import { createShapeFetch } from "./headers";
import { buildShapeRequestUrl, type ShapeQueryOptions } from "./query";
import type { ShapeName } from "./schema";
import type { ChannelRow, ChannelUserRow, MessageRow, UserRow } from "./types";

function shapeOptions(shape: ShapeName, config: AppConfig, options: ShapeQueryOptions = {}) {
  return {
    url: buildShapeRequestUrl(shape, config, options),
    fetchClient: createShapeFetch({ devUserId: config.devUserId }),
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

function channelUsersCollection(config: AppConfig) {
  const collection = createCollection(
    electricCollectionOptions<ChannelUserRow>({
      id: "channel_user",
      getKey: (row) => `${row.user_id}:${row.channel_id}`,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("channelUsers", config),
    }),
  );
  collection.createIndex((row) => row.user_id);
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
