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
  is_dm: boolean;
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
  parent_id: string | null;
}

export interface MessageReactionResponse {
  id: string;
  message_id: string;
  user_id: string;
  emoji: string;
  created_at: TimestampString;
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

export interface CreateDmRequest {
  id?: string;
  target_user_ids: string[];
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
  /** 返信先のルートメッセージ。通常メッセージでは省略する。 */
  parent_id?: string;
  attachment_ids?: string[];
}

export interface PresignAttachmentRequest {
  channel_id: string;
  file_name: string;
  file_size: number;
  content_type: string;
}

export interface PresignAttachmentResponse {
  attachment_id: string;
  upload_url: string;
  storage_key: string;
}

export interface AttachmentUrlResponse {
  id: string;
  download_url: string;
  file_name: string;
  file_size: number;
  content_type: string;
}

export interface UpdateMessageRequest {
  content: string;
}

export interface AddReactionRequest {
  id?: string;
  emoji: string;
}

export interface LeaveChannelResponse {
  user_id: string;
  channel_id: string;
}
