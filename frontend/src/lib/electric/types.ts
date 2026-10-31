import type { TimestampString } from "../timestamps";

export type TableName =
  | "users"
  | "channels"
  | "messages"
  | "message_reactions"
  | "channel_user"
  | "notifications"
  | "attachments";

export type UserRow = {
  id: string;
  user_name: string;
  display_name: string;
  is_bot?: boolean;
  app_id?: string | null;
  avatar_url?: string | null;
  bot_mentionable?: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
};

export type ChannelRow = {
  id: string;
  channel_name: string;
  description: string | null;
  created_user_id: string;
  is_private: boolean;
  is_dm: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  archive_at: TimestampString | null;
  deleted_at: TimestampString | null;
};

export type MessageRow = {
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  card?: string | null;
  app_blocks?: string | null;
  app_attachments?: string | null;
  app_unfurls?: string | null;
  app_origin_id?: string | null;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  deleted_at: TimestampString | null;
  /** 返信先のルートメッセージ。`null` はチャンネル直下のメッセージ。 */
  parent_id: string | null;
};

export type MessageReactionRow = {
  id: string;
  message_id: string;
  user_id: string;
  emoji: string;
  created_at: TimestampString;
};

export type ChannelUserRow = {
  user_id: string;
  channel_id: string;
  joined_at: TimestampString;
};

export type NotificationRow = {
  id: string;
  user_id: string;
  actor_id: string;
  channel_id: string;
  message_id: string;
  notification_type: string;
  is_read: boolean;
  created_at: TimestampString;
};

export type AttachmentRow = {
  id: string;
  message_id: string | null;
  uploader_id: string;
  channel_id: string;
  file_name: string;
  file_size: string | number;
  content_type: string;
  storage_key: string;
  created_at: TimestampString;
};

export type TableRowMap = {
  notifications: NotificationRow;
  users: UserRow;
  channels: ChannelRow;
  messages: MessageRow;
  message_reactions: MessageReactionRow;
  channel_user: ChannelUserRow;
  attachments: AttachmentRow;
};
