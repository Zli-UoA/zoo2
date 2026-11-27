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

/**
 * リンクが指すメッセージの ID。`?message=` を優先し、`?thread=` だけのリンクはスレッドの親を指す。
 * チャンネルそのものへのリンクは `null`。
 */
export function linkedMessageId(target: ChannelLinkTarget): string | null {
  return target.search.message ?? target.search.thread ?? null;
}

/**
 * 2 つの移動先が、同じチャンネル・メッセージ・スレッドを指すか。
 * URL の表記（エスケープや検索パラメーターの順）ではなく、ルートが読む値で比べる。
 */
export function sameChannelLinkTarget(left: ChannelLinkTarget, right: ChannelLinkTarget): boolean {
  return (
    left.channelId === right.channelId &&
    (left.search.message ?? null) === (right.search.message ?? null) &&
    (left.search.thread ?? null) === (right.search.thread ?? null)
  );
}

/**
 * ルーターの現在地や実際のリンクが生成した href を、チャンネル画面の移動先に戻す。
 * 相対 URL はアプリの origin で解決する。チャンネル画面ではない URL は `null`。
 */
export function channelLinkTargetFromHref(
  href: string | undefined,
  origin: string,
): ChannelLinkTarget | null {
  if (!href) return null;
  try {
    return parseChannelLink(new URL(href, origin).href, origin);
  } catch {
    return null;
  }
}

const MESSAGE_JUMP_EVENT = "zoo:jump-to-message";

/**
 * すでに URL の `?message=` が指している（表示中の）メッセージへのリンクを押したことを知らせる。
 * 移動先の URL が今と同じだと、ルーターは何も変えず、メッセージ一覧も再スクロールしない。
 * その場合に、離れた位置から対象の行へ戻すための合図。移動先が違うときは、通常の移動が行う。
 */
export function requestMessageJump(messageId: string): void {
  window.dispatchEvent(new CustomEvent(MESSAGE_JUMP_EVENT, { detail: messageId }));
}

/** {@link requestMessageJump} の合図を受け取る。戻り値で購読を解除する。 */
export function subscribeMessageJump(handler: (messageId: string) => void): () => void {
  const listener = (event: Event) => handler((event as CustomEvent<string>).detail);
  window.addEventListener(MESSAGE_JUMP_EVENT, listener);
  return () => window.removeEventListener(MESSAGE_JUMP_EVENT, listener);
}

const CHANNEL_LINK_REVEAL_EVENT = "zoo:reveal-current-channel-link";

/**
 * いま表示中の URL と同じチャンネル・メッセージ・スレッドを指すリンクを、通常の左クリックで押したことを知らせる。
 * URL が変わらないため、ルーターも履歴の blocker も動かない。ノートを表示している ChannelScreen は、これを受けて
 * 下書きの確認と保存中ガードを通してからメッセージタブへ戻す。異なる URL の移動前に発火すると、移動が拒否された
 * ときにも表示が変わってしまうので、呼び出し側は現在の URL と同じときだけ発火する。
 */
export function requestChannelLinkReveal(target: ChannelLinkTarget): void {
  window.dispatchEvent(
    new CustomEvent<ChannelLinkTarget>(CHANNEL_LINK_REVEAL_EVENT, { detail: target }),
  );
}

/**
 * 現在の URL と、クリックしたリンクが生成した href が同じ移動先を指すときだけ、
 * {@link requestChannelLinkReveal} を送る。異なる URL の移動前には送らない。
 */
export function requestChannelLinkRevealIfSameDestination(
  currentHref: string | undefined,
  clickedHref: string,
  origin = window.location.origin,
): void {
  const current = channelLinkTargetFromHref(currentHref, origin);
  const clicked = channelLinkTargetFromHref(clickedHref, origin);
  if (!current || !clicked || !sameChannelLinkTarget(current, clicked)) return;
  requestChannelLinkReveal(clicked);
}

/** {@link requestChannelLinkReveal} の合図を受け取る。戻り値で購読を解除する。 */
export function subscribeChannelLinkReveal(
  handler: (target: ChannelLinkTarget) => void,
): () => void {
  const listener = (event: Event) => handler((event as CustomEvent<ChannelLinkTarget>).detail);
  window.addEventListener(CHANNEL_LINK_REVEAL_EVENT, listener);
  return () => window.removeEventListener(CHANNEL_LINK_REVEAL_EVENT, listener);
}
