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
  is_nsfw: boolean;
  is_dm: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  archive_at: TimestampString | null;
  deleted_at: TimestampString | null;
  /** 暗号化チャンネル（docs/e2ee.md） */
  encrypted?: boolean;
  /** 暗号化チャンネルのメンバーと端末の世代（API では数値） */
  membership_epoch?: number;
}

export interface MessageResponse {
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  apub?: boolean;
  apub_visibility?: string | null;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  deleted_at: TimestampString | null;
  parent_id: string | null;
  forwarded_message_ids?: string[] | null;
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
  is_nsfw?: boolean;
  /** 暗号化チャンネルにする（`is_private: true` のときだけ。作成後は変えられない。docs/e2ee.md） */
  encrypted?: boolean;
}

export interface CreateDmRequest {
  id?: string;
  target_user_ids: string[];
  /** 暗号化 DM にする（1 対 1 は `edm:` の名前で get-or-create する。docs/e2ee.md） */
  encrypted?: boolean;
}

export interface UpdateChannelRequest {
  channel_name?: string;
  description?: string | null;
  is_private?: boolean;
  is_nsfw?: boolean;
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
  sensitive?: boolean;
  sensitive_attachment_ids?: string[];
  forwarded_message_ids?: string[];
}

export interface ForwardDmParticipant {
  id: string;
  user_name: string;
  display_name: string;
}

export interface ForwardAuthor {
  id: string;
  user_name: string;
  display_name: string;
  avatar_url?: string | null;
  is_bot?: boolean;
  app_id?: string | null;
}

export interface ResolvedForwardMessage {
  id: string;
  channel_id: string;
  channel_name: string;
  is_private: boolean;
  is_dm: boolean;
  dm_participants?: ForwardDmParticipant[];
  parent_id: string | null;
  user_id: string;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  content: string;
  card: string | null;
  components: string | null;
  message_kind: string | null;
  author: ForwardAuthor | null;
  emojis: Record<string, import("../electric/types").DisplayEmoji>;
  attachments: import("../electric/types").AttachmentRow[];
  has_forwards: boolean;
}

export type ForwardResolutionItem =
  | ({ status: "ok" } & ResolvedForwardMessage)
  | { status: "deleted" }
  | { status: "unavailable" };

export interface ResolveForwardsRequest {
  ids: string[];
}

export interface ResolveForwardsResponse {
  items: Record<string, ForwardResolutionItem>;
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

export type AttachmentUrlResponse = import("../imageMetadata").ImageMetadata & {
  id: string;
  download_url: string;
  thumbnail_url?: string;
  file_name: string;
  file_size: number;
  content_type: string;
};

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

/** ユーザーごとにピン留めしたチャンネル・DM の 1 行。 */
export interface UserPinnedChannelResponse {
  user_id: string;
  channel_id: string;
  position: number;
  /** アーカイブ中のチャンネルのピン。位置は保ち、表示だけを止める。 */
  archived?: boolean;
  created_at: TimestampString;
  updated_at: TimestampString;
}

export interface ChannelPinsResponse {
  pins: UserPinnedChannelResponse[];
  version: number;
}

export interface PinChannelResult {
  pin: UserPinnedChannelResponse;
  version: number;
  pins: UserPinnedChannelResponse[];
}

export interface UnpinChannelResult {
  removed: boolean;
  version: number;
  pins: UserPinnedChannelResponse[];
}

export interface ReorderChannelPinsResult {
  version: number;
  pins: UserPinnedChannelResponse[];
}
