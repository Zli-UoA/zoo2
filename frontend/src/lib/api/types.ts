import type { TimestampString } from "../timestamps";

export interface UserResponse {
  id: string;
  user_name: string;
  display_name: string;
  created_at: TimestampString;
  updated_at: TimestampString | null;
}

export interface ChannelResponse {
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

export interface MessageResponse {
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  deleted_at: TimestampString | null;
}

export interface ChannelUserResponse {
  user_id: string;
  channel_id: string;
  joined_at: TimestampString;
}

export interface CreateUserRequest {
  id?: string;
  user_name: string;
  display_name: string;
}

export interface CreateChannelRequest {
  id?: string;
  channel_name: string;
  description?: string | null;
  is_private?: boolean;
}

export interface UpdateChannelRequest {
  channel_name?: string;
  description?: string | null;
  is_private?: boolean;
}

export interface AddChannelUserRequest {
  user_id: string;
}

export interface CreateMessageRequest {
  id?: string;
  channel_id: string;
  content: string;
}

export interface UpdateMessageRequest {
  content: string;
}

export interface LeaveChannelResponse {
  user_id: string;
  channel_id: string;
}
