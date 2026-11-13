import type { TimestampString } from "../timestamps";

export type TableName =
  | "users"
  | "channels"
  | "messages"
  | "message_reactions"
  | "channel_user"
  | "channel_leave"
  | "notifications"
  | "attachments";

export type UserRow = {
  id: string;
  user_name: string;
  display_name: string;
  is_bot?: boolean;
  /** 無効化済み。Slack から取り込んだ未連携ユーザー (プレースホルダ) もここに入る */
  is_deactivated?: boolean;
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
  is_nsfw?: boolean;
  is_dm: boolean;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  archive_at: TimestampString | null;
  deleted_at: TimestampString | null;
  /** Slack から取り込んだチャンネル (取り込み先の新規作成も既存のチャンネルも) */
  imported_from_slack: boolean;
  /** Slack からの取り込みの種類 (新しく作った / 既存のチャンネルに追加)。取り込んでいなければ null */
  slack_import_kind: "created" | "existing" | null;
  /** 最後に Slack から取り込んだ日時 */
  slack_imported_at: TimestampString | null;
  /** 暗号化チャンネル（作成時だけ指定でき、変わらない。docs/e2ee.md） */
  encrypted?: boolean;
  /**
   * 暗号化チャンネルのメンバーと端末の世代。Electric は int8 を文字列で配るので、比べる前に BigInt などの
   * 正準な整数にする。
   */
  membership_epoch?: string;
};

export type MessageRow = {
  display_data?: MessageDisplayData;
  id: string;
  user_id: string;
  channel_id: string;
  content: string;
  card?: string | null;
  app_blocks?: string | null;
  app_attachments?: string | null;
  app_unfurls?: string | null;
  app_origin_id?: string | null;
  home_post_id?: string | null;
  apub?: boolean;
  apub_visibility?: string | null;
  created_at: TimestampString;
  updated_at: TimestampString | null;
  deleted_at: TimestampString | null;
  /** 返信先のルートメッセージ。`null` はチャンネル直下のメッセージ。 */
  parent_id: string | null;
  /** この投稿を返信先とするスレッドのアーカイブ日時。 */
  thread_archive_at?: TimestampString | null;
  /** 自動アーカイブ期限の計算に使う最終活動時刻。 */
  thread_activity_at?: TimestampString | null;
  /** 暗号化チャンネルの Megolm の暗号文（`m.room.encrypted` と同じ形）。平文と削除済みの行は `null`。 */
  ciphertext?: MegolmCiphertext | null;
  /** 暗号化したセッションの `membership_epoch`（int8 なので文字列）。 */
  epoch?: string | null;
  /** 編集の版（暗号化チャンネルでは編集のたびに 1 増える）。 */
  revision?: number;
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

/** 自分でチャンネルを退出した日時。本人の行だけが届く。 */
export type ChannelLeaveRow = {
  user_id: string;
  channel_id: string;
  left_at: TimestampString;
};

export type NotificationRow = {
  display_data?: NotificationDisplayData;
  id: string;
  user_id: string;
  actor_id: string;
  channel_id: string;
  message_id: string;
  notification_type: string;
  is_read: boolean;
  created_at: TimestampString;
};

export type DisplayEmoji = {
  image_id?: string | null;
  url: string;
  width?: number | null;
  height?: number | null;
};

/** 暗号化チャンネルのメッセージの暗号文（サーバーが形を検証する。docs/e2ee.md の「メッセージ」）。 */
export type MegolmCiphertext = {
  algorithm: "m.megolm.v1.aes-sha2";
  sender_key: string;
  device_id: string;
  session_id: string;
  ciphertext: string;
};

export type MessageDisplayData = {
  author?: Pick<UserRow, "id" | "user_name" | "display_name" | "avatar_url" | "is_bot" | "app_id">;
  emojis: Record<string, DisplayEmoji>;
  attachments: AttachmentRow[];
  reactions: (MessageReactionRow & { user_name?: string; display_name?: string })[];
  reply_count: number;
};

export type NotificationDisplayData = {
  message: {
    id: string;
    user_id: string;
    content: string;
    emojis?: Record<string, DisplayEmoji>;
  } | null;
  reactions: MessageDisplayData["reactions"];
  actor?: MessageDisplayData["author"];
  channel?: ChannelRow;
};

export type AttachmentRow = import("../imageMetadata").ImageMetadata & {
  id: string;
  message_id: string | null;
  uploader_id: string;
  channel_id: string;
  file_name: string;
  file_size: string | number;
  content_type: string;
  storage_key: string;
  is_sensitive?: boolean;
  created_at: TimestampString;
};

export type TableRowMap = {
  notifications: NotificationRow;
  users: UserRow;
  channels: ChannelRow;
  messages: MessageRow;
  message_reactions: MessageReactionRow;
  channel_user: ChannelUserRow;
  channel_leave: ChannelLeaveRow;
  attachments: AttachmentRow;
};
