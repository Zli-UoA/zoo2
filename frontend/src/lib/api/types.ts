import type { ChannelRow, ChannelUserRow, MessageRow, UserRow } from "../electric/types";

export type UserResponse = UserRow;
export type ChannelResponse = ChannelRow;
export type MessageResponse = MessageRow;
export type ChannelUserResponse = ChannelUserRow;

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
