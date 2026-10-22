import type { TimestampString } from "../timestamps";

export type TableName = "users" | "channels" | "messages" | "message_reactions" | "channel_user";

export type UserRow = {
  id: string;
  user_name: string;
  display_name: string;
  is_bot?: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
};

export type ChannelRow = {
  id: string;
  channel_name: string;
  description: string | null;
  created_user_id: string;
  is_private: boolean;
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

export type TableRowMap = {
  users: UserRow;
  channels: ChannelRow;
  messages: MessageRow;
  message_reactions: MessageReactionRow;
  channel_user: ChannelUserRow;
};
