import { useLinkedMessageRows } from "../../../hooks/useThread";
import { linkedMessageId, type ChannelLinkTarget } from "./messageLink";
import { messageExcerpt } from "./messageExcerpt";
import { authorLabel, type FeedMessage } from "./messageFormat";
import type { ChannelDirectory, LinkedChannel } from "./MfmChannels";

/** 埋め込みカードとリンクの短い表示に使う、リンク先のメッセージ。 */
export interface LinkedMessage {
  id: string;
  /** 返信なら、そのスレッドの親。`null` はチャンネル直下。 */
  parentId: string | null;
  authorName: string;
  avatarUrl: string | null;
  isBot: boolean;
  createdAt: string;
  /** 本文を素の文字列にした要約。暗号化チャンネルでは空（本文を出さない）。 */
  excerpt: string;
  attachmentCount: number;
}

/**
 * アプリ内リンクの行き先が、いま分かっていること。
 *
 * - `loading`: チャンネルの一覧かメッセージを取得中。「ない」とはまだ決めない。
 * - `unavailable`: 見られない。存在しない・削除済み、非公開で参加していない、はどれも同じ扱いで、
 *   理由を区別しない（存在を知らせない）。
 * - `channel` / `message`: 行き先が分かった。
 */
export type ChannelLinkInfo =
  | { status: "loading" }
  | { status: "unavailable"; subject: "channel" | "message" }
  | { status: "channel"; channel: LinkedChannel }
  | { status: "message"; channel: LinkedChannel; message: LinkedMessage };

type LinkedMessageRow = Pick<
  FeedMessage,
  | "id"
  | "user_id"
  | "content"
  | "parent_id"
  | "created_at"
  | "display_data"
  | "user_name"
  | "display_name"
  | "avatar_url"
  | "is_bot"
> & { deleted_at?: string | null };

export function toLinkedMessage(row: LinkedMessageRow, channel: LinkedChannel): LinkedMessage {
  return {
    id: row.id,
    parentId: row.parent_id,
    authorName: authorLabel(row as FeedMessage),
    avatarUrl: row.avatar_url ?? null,
    isBot: row.is_bot === true,
    createdAt: row.created_at,
    excerpt: channel.encrypted ? "" : messageExcerpt(row.content),
    attachmentCount: row.display_data?.attachments?.length ?? 0,
  };
}

/**
 * リンク先の状態を決める。チャンネルは、自分が見られるチャンネルの一覧（{@link ChannelDirectory}）で
 * 判定する。一覧にないチャンネル（削除済み、非公開で未参加、存在しない）のメッセージは、取得せずに
 * 「見られない」とする。
 *
 * `messages` は、メッセージを取得する問い合わせの結果。`undefined` は、まだ問い合わせていない。
 */
export function resolveChannelLinkInfo({
  directory,
  target,
  messages,
}: {
  directory: ChannelDirectory;
  target: ChannelLinkTarget;
  messages: { rows: readonly LinkedMessageRow[] | undefined; loading: boolean } | undefined;
}): ChannelLinkInfo {
  const messageId = linkedMessageId(target);
  const channel = directory.byId.get(target.channelId);
  if (!channel) {
    return directory.ready
      ? { status: "unavailable", subject: messageId ? "message" : "channel" }
      : { status: "loading" };
  }
  if (!messageId) return { status: "channel", channel };
  if (!messages || (messages.loading && !messages.rows?.length)) return { status: "loading" };
  const row = messages.rows?.find((candidate) => candidate.id === messageId);
  if (!row || row.deleted_at) return { status: "unavailable", subject: "message" };
  return { status: "message", channel, message: toLinkedMessage(row, channel) };
}

/**
 * リンク先の状態を購読する。{@link ChannelDirectory} のある画面（アプリのシェルの中）で使う。
 *
 * `enabled` が false の間は、メッセージを取得せず `loading` のまま。画面の近くに来た行だけが取得する。
 * メッセージを取得するのは、チャンネルが見られるときだけ。リアクション・添付の Shape は購読しない
 * （表示しないため）。
 */
export function useChannelLinkInfo(
  target: ChannelLinkTarget,
  directory: ChannelDirectory,
  enabled = true,
): ChannelLinkInfo {
  const messageId = linkedMessageId(target);
  const channel = directory.byId.get(target.channelId);
  const query = useLinkedMessageRows(enabled && channel ? channel.id : null, messageId);
  return resolveChannelLinkInfo({
    directory,
    target,
    messages:
      enabled && messageId
        ? { rows: query.data as readonly LinkedMessageRow[] | undefined, loading: query.isLoading }
        : undefined,
  });
}
