export type IsoTimestamp = string;

export type IntegerAsString = string;

export type TableName = "users" | "channels" | "messages" | "channel_user";

export interface UserRow {
  id: string;
  user_name: string;
  display_name: string;
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp | null;
}

export interface ChannelRow {
  id: string;
  channel_name: string;
  description: string | null;
  created_user_id: string;
  is_private: boolean;
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp | null;
  archive_at: IsoTimestamp | null;
  deleted_at: IsoTimestamp | null;
}

export interface MessageRow {
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  created_at: IsoTimestamp;
  updated_at: IsoTimestamp | null;
  deleted_at: IsoTimestamp | null;
}

export interface ChannelUserRow {
  user_id: string;
  channel_id: string;
  joined_at: IsoTimestamp;
}

export interface TableRowMap {
  users: UserRow;
  channels: ChannelRow;
  messages: MessageRow;
  channel_user: ChannelUserRow;
}

export type AnyRow = TableRowMap[TableName];
