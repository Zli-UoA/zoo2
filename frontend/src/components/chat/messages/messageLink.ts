import { defaultStringifySearch } from "@tanstack/react-router";

/** チャンネル画面（`/channels/$channelId`）が受け取る、メッセージへの移動先。 */
export interface MessageLinkTarget {
  channelId: string;
  search: { message: string; thread?: string };
}

/**
 * メッセージへ移動するための移動先。返信はスレッドを開いたうえで、その返信へ移動する。
 * 検索結果・コピーしたリンクなど、メッセージを指す経路はすべてこれを通す。
 */
export function messageLinkTarget(
  channelId: string,
  messageId: string,
  parentId: string | null,
): MessageLinkTarget {
  return {
    channelId,
    search: parentId ? { thread: parentId, message: messageId } : { message: messageId },
  };
}

/**
 * 共有用のメッセージリンク（絶対 URL）。ルーターが `navigate` で作る URL と同じ形式で、
 * 開くとチャンネル画面がそのメッセージへ移動する。ID だけを含み、本文は含まない。
 */
export function messageLinkUrl(
  origin: string,
  channelId: string,
  messageId: string,
  parentId: string | null,
): string {
  const target = messageLinkTarget(channelId, messageId, parentId);
  return `${origin}/channels/${encodeURIComponent(target.channelId)}${defaultStringifySearch(target.search)}`;
}
