import { useState } from "react";
import { useEncryptedDisplay } from "../../../hooks/useEncryptedDisplay";
import { useChannelHasCiphertext, useLinkedMessageRows } from "../../../hooks/useThread";
import { linkedMessageId, type ChannelLinkTarget } from "./messageLink";
import { messageBodySource } from "./messageBodySource";
import { messageExcerpt } from "./messageExcerpt";
import { authorLabel, type FeedMessage } from "./messageFormat";
import type { ChannelDirectory, LinkedChannel } from "./MfmChannels";

/**
 * 埋め込みカードの本文に、何を出すか。
 *
 * - `text`: 本文の要約（`excerpt`）。
 * - `encrypted`: 暗号化されたメッセージ。本文は出さない。
 * - `app`: 連携カード・部品・App のブロックの投稿。チャンネルでは `content` の代わりにそれらを描くので、
 *   `content` は出さず、短い代替の表示にする。
 */
export type LinkedMessageKind = "text" | "encrypted" | "app";

/** 埋め込みカードとリンクの短い表示に使う、リンク先のメッセージ。 */
export interface LinkedMessage {
  id: string;
  /** 返信なら、そのスレッドの親。`null` はチャンネル直下。 */
  parentId: string | null;
  authorName: string;
  avatarUrl: string | null;
  isBot: boolean;
  createdAt: string;
  kind: LinkedMessageKind;
  /** 本文の要約。`kind` が `text` のときだけ入る。 */
  excerpt: string;
  attachmentCount: number;
  /** 別のメッセージを転送（引用）した投稿か。暗号化されたメッセージは、常に false。 */
  forwarded: boolean;
}

/**
 * アプリ内リンクの行き先が、いま分かっていること。
 *
 * - `loading`: チャンネルの一覧かメッセージを取得中。「ない」とはまだ決めない。
 * - `unavailable`: 見られない。存在しない・削除済み、非公開で参加していない、はどれも同じ扱いで、
 *   理由を区別しない（存在を知らせない）。
 * - `failed`: メッセージの取得が、再試行されない形で失敗した（認証の期限切れなど）。見られるかは分からない。
 * - `channel` / `message`: 行き先が分かった。
 */
export type ChannelLinkInfo =
  | { status: "loading" }
  | { status: "unavailable"; subject: "channel" | "message" }
  | { status: "failed" }
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
  | "card"
  | "components"
  | "app_blocks"
  | "ciphertext"
  | "forwarded_message_ids"
> & { deleted_at?: string | null };

export function toLinkedMessage(row: LinkedMessageRow, channel: LinkedChannel): LinkedMessage {
  // 暗号文のある行は、チャンネルの `encrypted` がどうあっても暗号化されたメッセージとして扱う
  // （格下げされたチャンネルに、平文の `content` を差し込まれても、本文として出さない）。
  const kind: LinkedMessageKind =
    channel.encrypted || row.ciphertext != null
      ? "encrypted"
      : messageBodySource({
            components: row.components,
            appBlocks: row.app_blocks,
            card: row.card,
          }).kind === "text"
        ? "text"
        : "app";
  return {
    id: row.id,
    parentId: row.parent_id,
    authorName: authorLabel(row as FeedMessage),
    avatarUrl: row.avatar_url ?? null,
    isBot: row.is_bot === true,
    createdAt: row.created_at,
    kind,
    excerpt: kind === "text" ? messageExcerpt(row.content) : "",
    attachmentCount: row.display_data?.attachments?.length ?? 0,
    forwarded: kind !== "encrypted" && !!row.forwarded_message_ids?.length,
  };
}

/**
 * リンク先の状態を決める。チャンネルは、自分が見られるチャンネルの一覧（{@link ChannelDirectory}）で
 * 判定する。一覧にないチャンネル（削除済み、非公開で未参加、存在しない）のメッセージは、取得せずに
 * 「見られない」とする。
 *
 * `messages` は、メッセージを取得する問い合わせの結果。`undefined` は、まだ問い合わせていない。
 * 問い合わせが失敗（`failed`）して対象の行がないときは、「見られない」ではなく失敗とする。
 * `treatAsEncrypted` は、暗号化チャンネルとして扱うか（`useEncryptedDisplay`）。一覧の `encrypted` が
 * `false` でも（暗号化の格下げ）、true なら暗号化チャンネルとして扱う。
 * `checkingEncryption` は、暗号化かどうかを確かめている（確かめられなかった）か。その間は、平文の本文を
 * 出さず、読み込み中とする。
 */
export function resolveChannelLinkInfo({
  directory,
  target,
  messages,
  treatAsEncrypted = false,
  checkingEncryption = false,
}: {
  directory: ChannelDirectory;
  target: ChannelLinkTarget;
  messages:
    | { rows: readonly LinkedMessageRow[] | undefined; loading: boolean; failed?: boolean }
    | undefined;
  treatAsEncrypted?: boolean;
  checkingEncryption?: boolean;
}): ChannelLinkInfo {
  const messageId = linkedMessageId(target);
  const listed = directory.byId.get(target.channelId);
  if (!listed) {
    return directory.ready
      ? { status: "unavailable", subject: messageId ? "message" : "channel" }
      : { status: "loading" };
  }
  const channel = treatAsEncrypted && !listed.encrypted ? { ...listed, encrypted: true } : listed;
  if (!messageId) return { status: "channel", channel };
  if (!messages) return { status: "loading" };
  const row = messages.rows?.find((candidate) => candidate.id === messageId);
  if (!row && messages.failed) return { status: "failed" };
  if (messages.loading && !messages.rows?.length) return { status: "loading" };
  if (!row || row.deleted_at) return { status: "unavailable", subject: "message" };
  const message = toLinkedMessage(row, channel);
  // 暗号化かどうかが決まるまで、本文は出さない（格下げされたチャンネルに差し込まれた平文を、
  // 確認の前に一瞬でも出さないため）。本文を出さない種類（暗号化・App）は、待たずに出す。
  if (checkingEncryption && message.kind === "text") return { status: "loading" };
  return { status: "message", channel, message };
}

/**
 * リンク先の状態を購読する。{@link ChannelDirectory} のある画面（アプリのシェルの中）で使う。
 *
 * `info` は行き先の状態。`started` は、このリンク先の取得を始めたか（一度始めたら true のまま）。
 *
 * `enabled` が false の間は、メッセージを取得せず `loading` のまま。画面の近くに来た行だけが取得する。
 * 一度取得を始めたリンク先は、そのあと `enabled` が false に戻っても、表示を保つ
 * （画面から遠ざかったときに、表示が枠へ戻って、行の大きさが変わらないように）。リンク先が変わったら、
 * 新しいリンク先は、また `enabled` になるまで取得しない。
 * メッセージを取得するのは、チャンネルが見られるときだけ。リアクション・添付の Shape は購読しない
 * （表示しないため）。
 */
export function useChannelLinkInfo(
  target: ChannelLinkTarget,
  directory: ChannelDirectory,
  enabled = true,
): { info: ChannelLinkInfo; started: boolean } {
  const messageId = linkedMessageId(target);
  // 取得を始めたリンク先。リンク先が変わったら（同じ要素が別のリンクに使い回されたとき）、新しいリンク先は
  // 近くに来るまで取得しない。同じリンク先に戻ったときは、取得済みのものをそのまま使う。
  const targetKey = `${target.channelId}\n${messageId ?? ""}`;
  const [startedKeys, setStartedKeys] = useState<ReadonlySet<string>>(
    () => new Set(enabled ? [targetKey] : []),
  );
  if (enabled && !startedKeys.has(targetKey)) setStartedKeys(new Set(startedKeys).add(targetKey));
  const started = enabled || startedKeys.has(targetKey);
  const channel = directory.byId.get(target.channelId);
  // 同じチャンネルの別の行に暗号文があれば、その行が対象でなくても、暗号化チャンネルとして扱う
  // （`MessagePane` が、表示中の行で判定するのと同じ。読み込み済みの行だけを見て、行を取る要求は出さない）。
  // この購読は、そのチャンネルの `messages` の同期を始める。メッセージを取得する問い合わせ（下）が同じ
  // コレクションを始めるときだけ使い、チャンネルだけを指すリンクでは、同期を増やさないため見ない。
  const channelHasCiphertext = useChannelHasCiphertext(
    started && channel && messageId ? channel.id : null,
  );
  // 暗号化の判定（docs/e2ee.md の「暗号化の格下げを防ぐ」）は、`MessagePane` と同じ `useEncryptedDisplay` を
  // 使う。サーバーが `encrypted = false` を配っても、`localStorage` と暗号ストアの記録、`edm:` の名前、
  // 暗号文のある行で、暗号化チャンネルとして扱う。メッセージのリンクは、取得を始めてから確かめる。
  const { treatAsEncrypted, checkingEncryption } = useEncryptedDisplay({
    channelId: channel && (started || !messageId) ? channel.id : null,
    channelName: channel?.name,
    encrypted: channel?.encrypted,
    hasCiphertext: channelHasCiphertext,
    // 起動直後は暗号ストアを開いている途中。開き終わるまで本文を出さない（開けないと分かった状態では、
    // `localStorage` などの記録で表示する）。
    holdWhileOpening: true,
  });
  const query = useLinkedMessageRows(started && channel ? channel.id : null, messageId);
  const info = resolveChannelLinkInfo({
    directory,
    target,
    messages:
      started && messageId
        ? {
            rows: query.data as readonly LinkedMessageRow[] | undefined,
            loading: query.isLoading,
            failed: query.isError,
          }
        : undefined,
    treatAsEncrypted,
    checkingEncryption,
  });
  return { info, started };
}
