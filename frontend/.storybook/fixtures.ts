import { createCollection, localOnlyCollectionOptions } from "@tanstack/react-db";

import { DEFAULT_CONFIG, type AppConfig } from "../src/lib/config";
import type { CollectionsBundle } from "../src/lib/electric/collections";
import type {
  ChannelRow,
  ChannelUserRow,
  MessageReactionRow,
  MessageRow,
  NotificationRow,
  UserRow,
} from "../src/lib/electric/types";

export interface StoryCollectionsInput {
  currentUserId?: string;
  users?: UserRow[];
  channels?: ChannelRow[];
  channelUsers?: ChannelUserRow[];
  messages?: MessageRow[];
  reactions?: MessageReactionRow[];
  notifications?: NotificationRow[];
  config?: Partial<AppConfig>;
}

let fixtureSequence = 0;

function collectionId(label: string): string {
  fixtureSequence += 1;
  return `story:${label}:${fixtureSequence}`;
}

function rowsCollection<Row extends object>(
  label: string,
  getKey: (row: Row) => string,
  rows: Row[],
) {
  return createCollection(
    localOnlyCollectionOptions<Row>({
      id: collectionId(label),
      getKey,
      initialData: rows,
    }),
  );
}

function groupBy<Row>(rows: Row[], key: (row: Row) => string): Map<string, Row[]> {
  const grouped = new Map<string, Row[]>();
  for (const row of rows) {
    const value = key(row);
    const bucket = grouped.get(value);
    if (bucket) {
      bucket.push(row);
    } else {
      grouped.set(value, [row]);
    }
  }
  return grouped;
}

/**
 * Storybook / VRT 用のローカルコレクションを作る。
 * アプリの Electric コレクションと同じ公開 API（get / insert / update / delete /
 * live query）を満たすため、画面コンポーネントはそのまま描画できる。
 */
export function createStoryCollections(input: StoryCollectionsInput = {}): CollectionsBundle {
  const currentUserId = input.currentUserId ?? DEFAULT_CURRENT_USER_ID;
  const config: AppConfig = {
    ...DEFAULT_CONFIG,
    ...input.config,
    txidMode: "skip",
    devUserId: currentUserId,
  };

  const channels = rowsCollection<ChannelRow>("channels", (row) => row.id, input.channels ?? []);
  const users = rowsCollection<UserRow>("users", (row) => row.id, input.users ?? []);
  const channelUsers = rowsCollection<ChannelUserRow>(
    "channelUsers",
    (row) => `${row.user_id}:${row.channel_id}`,
    input.channelUsers ?? [],
  );

  const messagesByChannel = groupBy(input.messages ?? [], (row) => row.channel_id);
  const channelByMessage = new Map(
    (input.messages ?? []).map((message) => [message.id, message.channel_id]),
  );
  const reactionsByChannel = groupBy(
    input.reactions ?? [],
    (row) => channelByMessage.get(row.message_id) ?? "",
  );

  const messageCache = new Map<string, ReturnType<typeof createStoryMessageCollection>>();
  for (const channel of input.channels ?? []) {
    if (!messageCache.has(channel.id)) {
      messageCache.set(
        channel.id,
        createStoryMessageCollection(messagesByChannel.get(channel.id) ?? []),
      );
    }
  }

  const reactionCache = new Map<string, ReturnType<typeof createStoryReactionCollection>>();
  for (const channel of input.channels ?? []) {
    if (!reactionCache.has(channel.id)) {
      reactionCache.set(
        channel.id,
        createStoryReactionCollection(reactionsByChannel.get(channel.id) ?? []),
      );
    }
  }

  const notificationsByUser = groupBy(input.notifications ?? [], (row) => row.user_id);
  const notificationCache = new Map<string, ReturnType<typeof createStoryNotificationCollection>>();
  for (const userId of new Set((input.notifications ?? []).map((row) => row.user_id))) {
    notificationCache.set(
      userId,
      createStoryNotificationCollection(notificationsByUser.get(userId) ?? []),
    );
  }

  const bundle = {
    config,
    channels,
    users,
    channelUsers,
    notifications: (userId: string) => {
      let collection = notificationCache.get(userId);
      if (!collection) {
        collection = createStoryNotificationCollection([]);
        notificationCache.set(userId, collection);
      }
      return collection as unknown as ReturnType<CollectionsBundle["notifications"]>;
    },
    messages: (channelId: string) => {
      let collection = messageCache.get(channelId);
      if (!collection) {
        collection = createStoryMessageCollection([]);
        messageCache.set(channelId, collection);
      }
      return collection as unknown as ReturnType<CollectionsBundle["messages"]>;
    },
    reactions: (channelId: string) => {
      let collection = reactionCache.get(channelId);
      if (!collection) {
        collection = createStoryReactionCollection([]);
        reactionCache.set(channelId, collection);
      }
      return collection as unknown as ReturnType<CollectionsBundle["reactions"]>;
    },
  };

  return bundle as unknown as CollectionsBundle;
}

function createStoryMessageCollection(rows: MessageRow[]) {
  return rowsCollection<MessageRow>("messages", (row) => row.id, rows);
}

function createStoryReactionCollection(rows: MessageReactionRow[]) {
  return rowsCollection<MessageReactionRow>("reactions", (row) => row.id, rows);
}

function createStoryNotificationCollection(rows: NotificationRow[]) {
  return rowsCollection<NotificationRow>("notifications", (row) => row.id, rows);
}

export const DEFAULT_CURRENT_USER_ID = "user-alice";

const timestamp = (day: number, hour: number, minute = 0): string =>
  `2026-01-${String(day).padStart(2, "0")} ${String(hour).padStart(2, "0")}:${String(
    minute,
  ).padStart(2, "0")}:00.000`;

export const storyUsers: UserRow[] = [
  {
    id: "user-alice",
    user_name: "alice",
    display_name: "Alice",
    created_at: timestamp(1, 0),
    updated_at: null,
  },
  {
    id: "user-bob",
    user_name: "bob",
    display_name: "Bob",
    created_at: timestamp(1, 0),
    updated_at: null,
  },
  {
    id: "user-carol",
    user_name: "carol",
    display_name: "Carol",
    created_at: timestamp(1, 0),
    updated_at: null,
  },
  {
    id: "user-dave",
    user_name: "dave",
    display_name: "Dave",
    created_at: timestamp(1, 0),
    updated_at: null,
  },
  {
    id: "user-admin",
    user_name: "admin",
    display_name: "Admin",
    created_at: timestamp(1, 0),
    updated_at: null,
  },
];

export const storyChannels: ChannelRow[] = [
  {
    id: "channel-general",
    channel_name: "general",
    description: "チーム全体のお知らせと雑談",
    created_user_id: "user-alice",
    is_private: false,
    created_at: timestamp(1, 0),
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  },
  {
    id: "channel-design",
    channel_name: "design",
    description: "デザインの相談",
    created_user_id: "user-carol",
    is_private: false,
    created_at: timestamp(1, 0),
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  },
  {
    id: "channel-secret",
    channel_name: "secret",
    description: "非公開の作戦会議",
    created_user_id: "user-alice",
    is_private: true,
    created_at: timestamp(1, 0),
    updated_at: null,
    archive_at: null,
    deleted_at: null,
  },
  {
    id: "channel-archive",
    channel_name: "old-announcements",
    description: "アーカイブ済みのお知らせ",
    created_user_id: "user-admin",
    is_private: false,
    created_at: timestamp(1, 0),
    updated_at: null,
    archive_at: timestamp(1, 1),
    deleted_at: null,
  },
];

export const storyChannelUsers: ChannelUserRow[] = [
  { user_id: "user-alice", channel_id: "channel-general", joined_at: timestamp(1, 0) },
  { user_id: "user-bob", channel_id: "channel-general", joined_at: timestamp(1, 0) },
  { user_id: "user-carol", channel_id: "channel-general", joined_at: timestamp(1, 0) },
  { user_id: "user-alice", channel_id: "channel-design", joined_at: timestamp(1, 0) },
  { user_id: "user-carol", channel_id: "channel-design", joined_at: timestamp(1, 0) },
  { user_id: "user-dave", channel_id: "channel-design", joined_at: timestamp(1, 0) },
  { user_id: "user-alice", channel_id: "channel-secret", joined_at: timestamp(1, 0) },
  { user_id: "user-bob", channel_id: "channel-secret", joined_at: timestamp(1, 0) },
  { user_id: "user-admin", channel_id: "channel-general", joined_at: timestamp(1, 0) },
];

export const storyMessages: MessageRow[] = [
  {
    id: "message-hello",
    user_id: "user-alice",
    channel_id: "channel-general",
    content:
      "おはようございます。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。",
    created_at: timestamp(2, 9),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  },
  {
    id: "message-bob-reply",
    user_id: "user-bob",
    channel_id: "channel-general",
    content: "了解です！デプロイ後に確認します 👍",
    created_at: timestamp(2, 9, 2),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  },
  {
    id: "message-thread-root",
    user_id: "user-carol",
    channel_id: "channel-general",
    content: "リリースノートの下書きを共有しました。レビューをお願いします。",
    created_at: timestamp(2, 9, 5),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  },
  {
    id: "message-thread-reply",
    user_id: "user-bob",
    channel_id: "channel-general",
    content: "確認しました。2 点だけコメントしました。",
    created_at: timestamp(2, 9, 6),
    updated_at: null,
    deleted_at: null,
    parent_id: "message-thread-root",
  },
  {
    id: "message-thread-reply-2",
    user_id: "user-alice",
    channel_id: "channel-general",
    content: "ありがとうございます。修正して再共有します。",
    created_at: timestamp(2, 9, 7),
    updated_at: null,
    deleted_at: null,
    parent_id: "message-thread-root",
  },
  {
    id: "message-card",
    user_id: "user-alice",
    channel_id: "channel-general",
    content: "デプロイが完了しました",
    card: JSON.stringify({
      title: "Deploy complete",
      description: "production へのデプロイが完了しました。",
      source: "Deployment service",
      badge: "success",
    }),
    created_at: timestamp(2, 9, 10),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  },
  {
    id: "message-design",
    user_id: "user-carol",
    channel_id: "channel-design",
    content: "新しいカードのレイアウト案を Figma に追加しました。",
    created_at: timestamp(2, 10),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  },
];

export const storyReactions: MessageReactionRow[] = [
  {
    id: "reaction-hello-alice",
    message_id: "message-hello",
    user_id: "user-alice",
    emoji: "👍",
    created_at: timestamp(2, 9, 1),
  },
  {
    id: "reaction-hello-bob",
    message_id: "message-hello",
    user_id: "user-bob",
    emoji: "👍",
    created_at: timestamp(2, 9, 1),
  },
  {
    id: "reaction-hello-carol",
    message_id: "message-hello",
    user_id: "user-carol",
    emoji: "🎉",
    created_at: timestamp(2, 9, 1),
  },
  {
    id: "reaction-thread-kitchen",
    message_id: "message-thread-root",
    user_id: "user-bob",
    emoji: "kitchen:1f431_1f600",
    created_at: timestamp(2, 9, 6),
  },
  {
    id: "reaction-bob-alice",
    message_id: "message-bob-reply",
    user_id: "user-alice",
    emoji: "❤️",
    created_at: timestamp(2, 9, 3),
  },
];

export const storyNotifications: NotificationRow[] = [
  {
    id: "notification-mention",
    user_id: "user-alice",
    actor_id: "user-bob",
    channel_id: "channel-general",
    message_id: "message-hello",
    notification_type: "mention",
    is_read: false,
    created_at: timestamp(2, 9),
  },
  {
    id: "notification-reply",
    user_id: "user-alice",
    actor_id: "user-carol",
    channel_id: "channel-general",
    message_id: "message-thread-reply",
    notification_type: "reply",
    is_read: false,
    created_at: timestamp(2, 9, 6),
  },
  {
    id: "notification-reaction",
    user_id: "user-alice",
    actor_id: "user-bob",
    channel_id: "channel-general",
    message_id: "message-thread-root",
    notification_type: "reaction",
    is_read: false,
    created_at: timestamp(2, 9, 7),
  },
  {
    id: "notification-reaction-read",
    user_id: "user-alice",
    actor_id: "user-carol",
    channel_id: "channel-general",
    message_id: "message-bob-reply",
    notification_type: "reaction",
    is_read: true,
    created_at: timestamp(2, 9, 8),
  },
];

export const storyCollections = {
  currentUserId: DEFAULT_CURRENT_USER_ID,
  users: storyUsers,
  channels: storyChannels,
  channelUsers: storyChannelUsers,
  messages: storyMessages,
  reactions: storyReactions,
  notifications: storyNotifications,
} satisfies StoryCollectionsInput;
