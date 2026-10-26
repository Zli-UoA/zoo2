import { BasicIndex, createCollection } from "@tanstack/react-db";
import {
  persistedCollectionOptions,
  type PersistedCollectionPersistence,
} from "@tanstack/db-sqlite-persistence-core";
import { electricCollectionOptions } from "@tanstack/electric-db-collection";

import { createConfiguredApiClient } from "../api/client";
import { resolveConfig, type AppConfig } from "../config";
import { getCurrentUserStore } from "../currentUser";
import { OFFLINE_SCHEMA_VERSION } from "../offline/schema";
import { createShapeFetch, type ShapeFetchSource } from "./headers";
import { buildShapeRequestUrl, resolveShapeUrl, type ShapeQueryOptions } from "./query";
import { SHAPE_META, coerceElectricRow, type ShapeName } from "./schema";
import type {
  ChannelRow,
  ChannelUserRow,
  MessageReactionRow,
  MessageRow,
  NotificationRow,
  UserRow,
} from "./types";
import {
  createChannelUserWrites,
  createChannelWrites,
  createMessageWrites,
  createReactionWrites,
  createNotificationWrites,
  type WriteDependencies,
} from "./writes";

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
  const table = SHAPE_META[shape].table;
  return {
    url: resolveShapeUrl(buildShapeRequestUrl(shape, config, options), window.location.href),
    fetchClient: createShapeFetch(shapeAuthSource(config)),
    // 初期同期後は SSE で更新を受信し、空の long-poll を繰り返さない。
    // 認証・再接続・offset による再開は Electric のクライアントに任せる。
    liveSse: true,
    // Electric は integer を文字列、boolean を "true" / "false" で配信する。
    // そのままだと `"false"` が truthy になるなど型が崩れるため、Shape ごとの
    // スキーマで行を正規化してから TanStack DB に渡す。
    transformer: (row: Record<string, unknown>) => coerceElectricRow(table, row),
  };
}

type PersistentRow = ChannelRow | UserRow | ChannelUserRow | MessageRow | NotificationRow;

/**
 * Electric のコレクションを、指定があれば TanStack DB の SQLite 永続化で包む。
 *
 * `schemaMismatchPolicy` は永続化側の設定（`sync-present-reset`）に任せ、
 * スキーマ変更時は古いキャッシュを捨てて Electric から取り直す。
 * `initialRender` を付けると、ネットワークより先に復元済みの行を描画できる。
 */
function withPersistence<
  T extends PersistentRow,
  TKey extends string | number,
  TOptions extends object,
>(
  options: TOptions,
  persistence: PersistedCollectionPersistence | null | undefined,
  getKey: (row: T) => TKey,
): TOptions {
  if (!persistence) {
    return options;
  }
  // 永続化ラッパーは型に schema / utils を足すが、アプリ側の公開型は
  // 従来の Electric コレクションのまま扱う。
  return persistedCollectionOptions({
    ...options,
    getKey,
    persistence,
    schemaVersion: OFFLINE_SCHEMA_VERSION,
    initialRender: { strategy: "network-first", networkTimeoutMs: 2_000 },
  } as unknown as Parameters<typeof persistedCollectionOptions>[0]) as unknown as TOptions;
}

function channelsCollection(
  config: AppConfig,
  dependencies: WriteDependencies,
  persistence?: PersistedCollectionPersistence | null,
) {
  const writes = createChannelWrites(dependencies);
  const collection = createCollection(
    withPersistence(
      electricCollectionOptions<ChannelRow>({
        id: "channels",
        getKey: (row) => row.id,
        defaultIndexType: BasicIndex,
        shapeOptions: shapeOptions("channels", config),
        onInsert: writes.onInsert,
        onUpdate: writes.onUpdate,
      }),
      persistence,
      (row: ChannelRow) => row.id,
    ),
  );
  collection.createIndex((row) => row.id);
  return collection;
}

function usersCollection(config: AppConfig, persistence?: PersistedCollectionPersistence | null) {
  const collection = createCollection(
    withPersistence(
      electricCollectionOptions<UserRow>({
        id: "users",
        getKey: (row) => row.id,
        defaultIndexType: BasicIndex,
        shapeOptions: shapeOptions("users", config),
      }),
      persistence,
      (row: UserRow) => row.id,
    ),
  );
  // messages ⋈ users の結合キー。live query の警告を避けるため索引を張る。
  collection.createIndex((row) => row.id);
  return collection;
}

/**
 * メンバー一覧用の channel_user。今は `channel_id` を指定せず購読し、
 * `useChannelMembers` がチャンネル単位で絞り込む。`shapeQueryParams` の
 * `channelUsers` + `channelId` 分岐は、将来チャンネル単位で購読する場合用。
 *
 * proxy で `channel_id` なしの場合、バックエンドは channel_user のサブクエリ経路を
 * 使う（`channel_id IN (SELECT ...)`）。`ELECTRIC_FEATURE_FLAGS=allow_subqueries`
 * の Electric 1.8.1 で `params[1]` 付きでも通ることは Phase 5 で確認済み
 * （docs/electric-sync-requirements.md §1）。
 */
function channelUsersCollection(
  config: AppConfig,
  dependencies: WriteDependencies,
  persistence?: PersistedCollectionPersistence | null,
) {
  const writes = createChannelUserWrites(dependencies);
  const collection = createCollection(
    withPersistence(
      electricCollectionOptions<ChannelUserRow>({
        id: "channel_user",
        getKey: (row) => `${row.user_id}:${row.channel_id}`,
        defaultIndexType: BasicIndex,
        shapeOptions: shapeOptions("channelUsers", config),
        onInsert: writes.onInsert,
        onDelete: writes.onDelete,
      }),
      persistence,
      (row: ChannelUserRow) => `${row.user_id}:${row.channel_id}`,
    ),
  );
  // channel_id 単位に絞り込む `useChannelMembers` の述語に合わせて索引を張る。
  collection.createIndex((row) => row.channel_id);
  return collection;
}

function messagesCollection(
  channelId: string,
  config: AppConfig,
  dependencies: WriteDependencies,
  persistence?: PersistedCollectionPersistence | null,
) {
  const writes = createMessageWrites(dependencies);
  const collection = createCollection(
    withPersistence(
      electricCollectionOptions<MessageRow>({
        id: `messages:${channelId}`,
        getKey: (row) => row.id,
        defaultIndexType: BasicIndex,
        shapeOptions: shapeOptions("messages", config, { channelId }),
        onInsert: writes.onInsert,
        onUpdate: writes.onUpdate,
        onDelete: writes.onDelete,
      }),
      persistence,
      (row: MessageRow) => row.id,
    ),
  );
  collection.createIndex((row) => row.id);
  return collection;
}

function reactionsCollection(
  channelId: string,
  config: AppConfig,
  dependencies: WriteDependencies,
) {
  const writes = createReactionWrites(dependencies);
  const collection = createCollection(
    electricCollectionOptions<MessageReactionRow>({
      id: `message_reactions:${channelId}`,
      getKey: (row) => row.id,
      defaultIndexType: BasicIndex,
      shapeOptions: shapeOptions("messageReactions", config, { channelId }),
      onInsert: writes.onInsert,
      onDelete: writes.onDelete,
    }),
  );
  // メッセージ単位のチップ集計と、自分のリアクション判定で引く。
  collection.createIndex((row) => row.message_id);
  collection.createIndex((row) => row.user_id);
  return collection;
}

function notificationsCollection(
  userId: string,
  config: AppConfig,
  dependencies: WriteDependencies,
  persistence?: PersistedCollectionPersistence | null,
) {
  const writes = createNotificationWrites(dependencies);
  return createCollection(
    withPersistence(
      electricCollectionOptions<NotificationRow>({
        id: `notifications:${userId}`,
        getKey: (row) => row.id,
        defaultIndexType: BasicIndex,
        shapeOptions: shapeOptions("notifications", config, { userId }),
        onUpdate: writes.onUpdate,
      }),
      persistence,
      (row: NotificationRow) => row.id,
    ),
  );
}

export type NotificationsCollection = ReturnType<typeof notificationsCollection>;
export type ChannelsCollection = ReturnType<typeof channelsCollection>;
export type UsersCollection = ReturnType<typeof usersCollection>;
export type ChannelUsersCollection = ReturnType<typeof channelUsersCollection>;
export type MessagesCollection = ReturnType<typeof messagesCollection>;
export type ReactionsCollection = ReturnType<typeof reactionsCollection>;

export interface CollectionsBundle {
  config: AppConfig;
  /** 選択された永続化。null ならメモリのみ（OPFS が使えない環境）。 */
  persistence: PersistedCollectionPersistence | null;
  /** ユーザー単位の OPFS 名前空間。 */
  namespace: string;
  notifications: (userId: string) => NotificationsCollection;
  channels: ChannelsCollection;
  users: UsersCollection;
  channelUsers: ChannelUsersCollection;
  /** `channel_id` 単位で生成しキャッシュした messages コレクションを返す。 */
  messages: (channelId: string) => MessagesCollection;
  /** `channel_id` 単位で生成しキャッシュした message_reactions コレクションを返す。 */
  reactions: (channelId: string) => ReactionsCollection;
}

export function isBrowser(): boolean {
  return typeof window !== "undefined";
}

let resolvedConfig: AppConfig | null = null;
interface CachedBundle {
  config: AppConfig;
  persistence: PersistedCollectionPersistence | null;
  bundle: CollectionsBundle;
}
const bundles = new Map<string, CachedBundle>();

function defaultConfig(): AppConfig {
  resolvedConfig ??= resolveConfig();
  return resolvedConfig;
}

export interface CollectionsOptions {
  /** SQLite 永続化。null / 未指定ならメモリのみ。 */
  persistence?: PersistedCollectionPersistence | null;
  /** ユーザー単位の名前空間（キャッシュのキー）。 */
  namespace?: string;
}

/**
 * ブラウザ専用のコレクションバンドルを返す（SSR では呼ばない）。
 *
 * Electric の ShapeStream はブラウザ API に依存するため、モジュール直下ではなく
 * client コンポーネント / フックから呼び出す。messages だけは channel_id 単位で
 * 遅延生成してキャッシュする。
 */
export function getCollections(
  config?: AppConfig,
  options: CollectionsOptions = {},
): CollectionsBundle {
  if (!isBrowser()) {
    throw new Error("Electric collections can only be created in the browser");
  }
  // ブラウザ判定の後で設定を解決する。dev 用の必須項目（VITE_DEV_USER_ID）が
  // 未設定でも、SSR / テストでは設定エラーではなく「ブラウザ専用」を返す。
  const resolved = config ?? defaultConfig();
  const persistence = options.persistence ?? null;
  const namespace = options.namespace ?? "default";
  const cached = bundles.get(namespace);
  if (cached && cached.config === resolved && cached.persistence === persistence) {
    return cached.bundle;
  }

  const notificationCache = new Map<string, NotificationsCollection>();
  const messageCache = new Map<string, MessagesCollection>();
  const reactionCache = new Map<string, ReactionsCollection>();
  // 現在ユーザーは `useCurrentUser` と同じストアから引く（OIDC では
  // `/api/users/me` の解決後に値が入る）。退出の本人確認は書き込み時に
  // getCurrentUserId() で最新値を読む。
  const currentUserStore = getCurrentUserStore(resolved);
  const dependencies: WriteDependencies = {
    client: createConfiguredApiClient(resolved),
    txidMode: resolved.txidMode,
    getCurrentUserId: () => currentUserStore.getState().currentUserId,
  };
  const bundle: CollectionsBundle = {
    config: resolved,
    persistence,
    namespace,
    notifications: (userId) => {
      let collection = notificationCache.get(userId);
      if (!collection) {
        collection = notificationsCollection(userId, resolved, dependencies, persistence);
        notificationCache.set(userId, collection);
      }
      return collection;
    },
    channels: channelsCollection(resolved, dependencies, persistence),
    users: usersCollection(resolved, persistence),
    channelUsers: channelUsersCollection(resolved, dependencies, persistence),
    reactions: (channelId) => {
      let collection = reactionCache.get(channelId);
      if (!collection) {
        collection = reactionsCollection(channelId, resolved, dependencies);
        reactionCache.set(channelId, collection);
      }
      return collection;
    },
    messages: (channelId) => {
      let collection = messageCache.get(channelId);
      if (!collection) {
        collection = messagesCollection(channelId, resolved, dependencies, persistence);
        messageCache.set(channelId, collection);
      }
      return collection;
    },
  };
  bundles.set(namespace, { config: resolved, persistence, bundle });
  return bundle;
}

/** ログアウト後に、その名前空間のコレクションをキャッシュから外す。 */
export function forgetCollections(namespace: string): void {
  bundles.delete(namespace);
}
