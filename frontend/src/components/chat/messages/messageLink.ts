import { defaultParseSearch, defaultStringifySearch } from "@tanstack/react-router";

/** チャンネル画面（`/channels/$channelId`）が受け取る、メッセージへの移動先。 */
export interface MessageLinkTarget {
  channelId: string;
  search: { message: string; thread?: string };
}

/** チャンネル画面が受け取る移動先。`message` も `thread` もなければ、チャンネルそのものを指す。 */
export interface ChannelLinkTarget {
  channelId: string;
  search: { message?: string; thread?: string };
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

/**
 * 本文中の URL が、このアプリのチャンネルまたはメッセージへのリンク（`/channels/<ID>`、
 * メッセージリンクは `?message=` / `?thread=` 付き）なら、アプリ内の移動先を返す。
 * `messageLinkUrl` の逆で、チャンネル画面のルートと同じく `message` / `thread` だけを読む。
 *
 * 別の origin、別のパス、ユーザー情報付きの URL は `null`（外部リンクとして扱う）。
 * 認証・API・アセットなど、チャンネル画面ではない同じ origin の URL を誤って奪わないよう、
 * 受け付けるパスは `/channels/<ID>` だけに限る。
 */
export function parseChannelLink(href: string, origin: string): ChannelLinkTarget | null {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.origin !== origin || url.username || url.password) return null;
  const match = /^\/channels\/([^/]+)\/?$/.exec(url.pathname);
  if (!match) return null;
  let channelId: string;
  try {
    channelId = decodeURIComponent(match[1]!);
  } catch {
    return null;
  }
  const params = defaultParseSearch(url.search) as Record<string, unknown>;
  const message = typeof params.message === "string" && params.message ? params.message : null;
  const thread = typeof params.thread === "string" && params.thread ? params.thread : null;
  return {
    channelId,
    search: {
      ...(thread ? { thread } : {}),
      ...(message ? { message } : {}),
    },
  };
}
