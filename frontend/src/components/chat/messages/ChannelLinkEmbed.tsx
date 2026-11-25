import {
  BlocksIcon,
  CloudOffIcon,
  EyeOffIcon,
  ForwardIcon,
  LockKeyholeIcon,
  PaperclipIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "cn";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { useLocale } from "../../i18n/LocaleProvider";
import { AppAvatar } from "../AppIdentity";
import { useChannelLinkInfo, type LinkedMessage } from "./channelLinkInfo";
import { useChannelLabel } from "./channelLinkLabel";
import { ChannelLink } from "./ContentLink";
import { fullTimestamp, formatEmbedTimestamp } from "./messageFormat";
import type { ChannelLinkTarget } from "./messageLink";
import type { ChannelDirectory, LinkedChannel } from "./MfmChannels";

const cardClassName =
  "my-2 flex w-full max-w-md min-w-0 gap-3 rounded-lg border bg-card p-3 text-sm";
const linkCardClassName = cn(
  cardClassName,
  "transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring outline-none",
);

function MessageEmbed({
  target,
  channel,
  message,
}: {
  target: ChannelLinkTarget;
  channel: LinkedChannel;
  message: LinkedMessage;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const channelLabel = useChannelLabel()(channel);
  return (
    <ChannelLink target={target} className={linkCardClassName}>
      <AppAvatar
        label={message.authorName}
        isBot={message.isBot}
        src={message.avatarUrl}
        size="sm"
        className="mt-0.5"
        aria-hidden
      />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="truncate font-semibold text-foreground">{message.authorName}</span>
          <time
            dateTime={normalizeUtcTimestamp(message.createdAt)}
            title={fullTimestamp(message.createdAt, locale)}
            className="shrink-0 text-xs text-muted-foreground"
          >
            {formatEmbedTimestamp(message.createdAt, locale)}
          </time>
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {message.parentId ? t("channelLink.thread", { channel: channelLabel }) : channelLabel}
        </span>
        <MessageEmbedBody message={message} />
      </span>
    </ChannelLink>
  );
}

function MessageEmbedBody({ message }: { message: LinkedMessage }) {
  const { t } = useTranslation();
  if (message.kind === "encrypted") {
    return (
      <span className="mt-1 flex items-center gap-1.5 text-muted-foreground italic">
        <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("channelLink.encrypted")}</span>
      </span>
    );
  }
  if (message.kind === "app") {
    // チャンネルでは、`content` の代わりにカード・部品を描く投稿。その代わりの表示は作らず、種類だけを伝える。
    return (
      <span className="mt-1 flex items-center gap-1.5 text-muted-foreground">
        <BlocksIcon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("channelLink.appMessage")}</span>
      </span>
    );
  }
  if (message.excerpt) {
    return (
      <span className="mt-1 line-clamp-3 break-words text-foreground [overflow-wrap:anywhere]">
        {message.excerpt}
      </span>
    );
  }
  if (message.forwarded) {
    // コメントのない転送。転送元は展開せず、通知と同じ文言で、転送したことだけを伝える。
    return (
      <span className="mt-1 flex items-center gap-1.5 text-muted-foreground">
        <ForwardIcon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("notifications.forwardedPreview")}</span>
      </span>
    );
  }
  if (message.attachmentCount > 0) {
    return (
      <span className="mt-1 flex items-center gap-1.5 text-muted-foreground">
        <PaperclipIcon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("channelLink.attachments", { count: message.attachmentCount })}</span>
      </span>
    );
  }
  return null;
}

function ChannelEmbed({ target, channel }: { target: ChannelLinkTarget; channel: LinkedChannel }) {
  const { t } = useTranslation();
  const channelLabel = useChannelLabel()(channel);
  const note = channel.encrypted
    ? t("channelLink.encryptedChannel")
    : channel.isPrivate && !channel.isDm
      ? t("channelLink.privateChannel")
      : null;
  return (
    <ChannelLink target={target} className={linkCardClassName}>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate font-semibold text-foreground">{channelLabel}</span>
        {note ? (
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <LockKeyholeIcon className="size-3 shrink-0" aria-hidden="true" />
            <span>{note}</span>
          </span>
        ) : null}
        {/* 暗号化チャンネルでも、説明はチャンネルの設定で、メッセージの本文ではない。 */}
        {channel.description ? (
          <span className="mt-1 line-clamp-2 break-words text-muted-foreground [overflow-wrap:anywhere]">
            {channel.description}
          </span>
        ) : null}
      </span>
    </ChannelLink>
  );
}

/** 見られない場合とは区別する。行き先が見られるかは、分からない。 */
function LoadFailed() {
  const { t } = useTranslation();
  return (
    <div className={cn(cardClassName, "items-center text-muted-foreground")}>
      <CloudOffIcon className="size-4 shrink-0" aria-hidden="true" />
      <span>{t("channelLink.loadFailed")}</span>
    </div>
  );
}

/**
 * 取得を始めてから、これだけたっても終わらないときは、読み込めなかったものとして枠をやめる。
 * サーバーのエラー（5xx）やオフラインでは、Electric のクライアントが再試行を続け、`useLiveQuery` は
 * `isLoading` のままで、失敗を状態から知れない。あとで取得できれば、そのまま内容に切り替わる。
 * 再試行されない失敗（4xx）は `isError` になるので、待たずに `failed` とする。
 */
export const LINK_LOAD_TIMEOUT_MS = 10_000;

/** `pending` が続いて `ms` たったか。`pending` でなくなったら戻す。 */
function useStalled(pending: boolean, ms: number): boolean {
  const [stalled, setStalled] = useState(false);
  useEffect(() => {
    if (!pending) return;
    const timer = setTimeout(() => setStalled(true), ms);
    return () => {
      clearTimeout(timer);
      setStalled(false);
    };
  }, [pending, ms]);
  return pending && stalled;
}

/**
 * このアプリのチャンネル・メッセージへのリンクの埋め込みカード。カードのどこを押しても、同じタブで
 * そのメッセージ・チャンネルへ移動する。
 *
 * 行き先を取得して表示するので、Summaly には送らない。見られない行き先（存在しない・削除済み、
 * 非公開で未参加）は理由を区別せず、「表示できません」とだけ出し、移動もしない。
 * 暗号化チャンネルのメッセージ（格下げされたチャンネルや、暗号文のある行を含む）は本文を出さず、暗号化されて
 * いることだけを伝える。連携カード・部品の投稿は `content` を出さず、種類だけを伝える。
 *
 * `active` が false の間は取得せず、場所だけを確保する。`showFallback` が false のときは、
 * 行き先が分かった場合だけ出す。
 */
export function ChannelLinkEmbed({
  target,
  directory,
  active = true,
  showFallback = true,
}: {
  target: ChannelLinkTarget;
  directory: ChannelDirectory;
  active?: boolean;
  showFallback?: boolean;
}) {
  const { t } = useTranslation();
  const info = useChannelLinkInfo(target, directory, active);
  // 数えるのは、リンク先のメッセージの取得（と、暗号化かどうかの確認）を待っている間だけ。取得を始めて
  // いない（画面から遠い）間と、チャンネルの一覧の同期を待っている間（一覧にないチャンネルは、まだ取得を
  // 始められない）は数えない。
  const fetchingMessage =
    info.status === "loading" && active && directory.byId.has(target.channelId);
  const stalled = useStalled(fetchingMessage, LINK_LOAD_TIMEOUT_MS);
  switch (info.status) {
    case "loading":
      if (!showFallback) return null;
      return stalled ? (
        <LoadFailed />
      ) : (
        // 読み込み中は、行の高さが後から大きく動かないよう、カードの形だけを置く。
        <div
          role="status"
          aria-busy="true"
          className={cn(cardClassName, "h-[5.25rem] animate-pulse bg-muted/40")}
        >
          <span className="sr-only">{t("channelLink.loading")}</span>
        </div>
      );
    case "failed":
      return showFallback ? <LoadFailed /> : null;
    case "unavailable":
      return showFallback ? (
        <div className={cn(cardClassName, "items-center text-muted-foreground")}>
          <EyeOffIcon className="size-4 shrink-0" aria-hidden="true" />
          <span>
            {info.subject === "message"
              ? t("channelLink.messageUnavailable")
              : t("channelLink.channelUnavailable")}
          </span>
        </div>
      ) : null;
    case "channel":
      return <ChannelEmbed target={target} channel={info.channel} />;
    case "message":
      return <MessageEmbed target={target} channel={info.channel} message={info.message} />;
  }
}
