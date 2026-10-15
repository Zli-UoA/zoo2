import type { TimestampString } from "../timestamps";

export type TableName = "users" | "channels" | "messages" | "channel_user";

export interface UserRow {
  id: string;
  user_name: string;
  display_name: string;
  created_at: TimestampString;
  updated_at: TimestampString | null;
}

export interface ChannelRow {
  id: string;
  channel_name: string;
  description: string | null;
  created_user_id: string;
  is_private: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  archive_at: TimestampString | null;
  deleted_at: TimestampString | null;
}

export interface MessageRow {
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  deleted_at: TimestampString | null;
}

export interface ChannelUserRow {
  user_id: string;
  channel_id: string;
  joined_at: TimestampString;
}

export interface TableRowMap {
  users: UserRow;
  channels: ChannelRow;
  messages: MessageRow;
  channel_user: ChannelUserRow;
}
