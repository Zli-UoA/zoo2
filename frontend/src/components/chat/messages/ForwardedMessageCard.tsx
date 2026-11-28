import { Suspense, lazy, type ReactNode } from "react";
import { useRouter } from "@tanstack/react-router";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import {
  CircleSlash2Icon,
  ForwardIcon,
  LockKeyholeIcon,
  PaperclipIcon,
  RefreshCwIcon,
} from "lucide-react";

import {
  useForwardedMessagesContext,
  type ForwardEntryState,
} from "../../../hooks/useForwardedMessages";
import type { ResolvedForwardMessage } from "../../../lib/api/types";
import type { Locale } from "../../../lib/i18n/locale";
import { normalizeUtcTimestamp } from "../../../lib/timestamps";
import { useLocale } from "../../i18n/LocaleProvider";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { AppAvatar, openApp } from "../AppIdentity";
import { hidesContent, parseComponents } from "../components/parse";
import { formatDmDisplayName, resolveDmDisplayName } from "../dm";
import { ChannelLink } from "./ContentLink";
import { EditedIndicator, MessageBody } from "./MessageBody";
import { MessageAttachments } from "./MessageAttachments";
import { formatTime, fullTimestamp } from "./messageFormat";
import { messageLinkTarget, messageLinkUrl } from "./messageLink";

const ComponentTree = lazy(() =>
  import("../components/ComponentTree").then((m) => ({ default: m.ComponentTree })),
);

function ForwardJumpLink({
  channelId,
  messageId,
  parentId,
  className,
  children,
}: {
  channelId: string;
  messageId: string;
  parentId: string | null;
  className?: string;
  children: ReactNode;
}) {
  const router = useRouter({ warn: false });
  if (!router) {
    const origin = typeof window !== "undefined" ? window.location.origin : "https://zoo.invalid";
    return (
      <a href={messageLinkUrl(origin, channelId, messageId, parentId)} className={className}>
        {children}
      </a>
    );
  }
  return (
    <ChannelLink target={messageLinkTarget(channelId, messageId, parentId)} className={className}>
      {children}
    </ChannelLink>
  );
}

function formatForwardChannelLabel(
  message: ResolvedForwardMessage,
  currentUserId: string | null,
  locale: Locale,
  t: TFunction,
): string {
  if (!message.is_dm) {
    return `#${message.channel_name}`;
  }
  const participants = message.dm_participants ?? [];
  const resolved = resolveDmDisplayName({
    channelName: message.channel_name,
    members: participants.map((p) => ({
      user_id: p.id,
      channel_id: message.channel_id,
      joined_at: "",
    })),
    users: participants.map((p) => ({
      id: p.id,
      user_name: p.user_name,
      display_name: p.display_name,
    })),
    currentUserId,
    locale,
  });
  return formatDmDisplayName(resolved, t);
}

export interface ForwardedMessageCardProps {
  messageId: string;
  /** ストーリーや確認プレビュー向けに直接解決済みの状態を渡す。未指定時は Provider から取得。 */
  entry?: ForwardEntryState;
  onRetry?: (messageId: string) => void;
  /** 転送ダイアログ内プレビューなど、簡潔な表示にする場合は true。 */
  compact?: boolean;
}

/** 転送（引用）元メッセージカード。閲覧権限と削除状態を反映し、元投稿へのジャンプを提供する。 */
export function ForwardedMessageCard({
  messageId,
  entry,
  onRetry,
  compact = false,
}: ForwardedMessageCardProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const context = useForwardedMessagesContext();
  const resolved = entry ?? context.entries.get(messageId) ?? { status: "loading" as const };
  const handleRetry = onRetry ?? context.retry;

  if (resolved.status === "loading") {
    return (
      <div
        className="mt-1.5 max-w-2xl rounded-r-lg border border-l-4 border-l-muted-foreground/40 bg-muted/30 px-3 py-2"
        aria-busy="true"
        aria-label={t("messages.forwardLoading")}
      >
        <div className="flex items-center gap-2">
          <Skeleton className="size-5 rounded-full" />
          <Skeleton className="h-3.5 w-24" />
        </div>
        <Skeleton className="mt-1.5 h-3.5 w-52" />
      </div>
    );
  }

  if (resolved.status === "deleted") {
    return (
      <div className="mt-1.5 flex max-w-2xl items-center gap-2 rounded-r-lg border border-l-4 border-l-muted-foreground/40 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <CircleSlash2Icon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("messages.forwardDeleted")}</span>
      </div>
    );
  }

  if (resolved.status === "unavailable") {
    return (
      <div className="mt-1.5 flex max-w-2xl items-center gap-2 rounded-r-lg border border-l-4 border-l-muted-foreground/40 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <LockKeyholeIcon className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{t("messages.forwardUnavailable")}</span>
      </div>
    );
  }

  if (resolved.status === "error") {
    return (
      <div className="mt-1.5 flex max-w-2xl flex-wrap items-center justify-between gap-2 rounded-r-lg border border-l-4 border-destructive/40 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
        <span>{t("messages.forwardLoadError")}</span>
        <Button type="button" variant="ghost" size="xs" onClick={() => handleRetry(messageId)}>
          <RefreshCwIcon className="size-3" aria-hidden="true" />
          {t("offline.retry")}
        </Button>
      </div>
    );
  }

  const authorName =
    resolved.author?.display_name || resolved.author?.user_name || resolved.user_id;
  const channelLabel = formatForwardChannelLabel(resolved, context.currentUserId, locale, t);
  const doc = parseComponents(resolved.components);
  const bodyHidden = hidesContent(doc) || (!resolved.content && !resolved.card);
  const sourceChannelText = resolved.parent_id
    ? t("messages.forwardSourceThread", { channel: channelLabel })
    : t("messages.forwardSourceChannel", { channel: channelLabel });

  return (
    <blockquote
      className="mt-1.5 max-w-2xl rounded-r-lg border border-l-4 border-l-brand/60 bg-muted/30 px-3 py-2"
      aria-label={t("messages.forwardQuoteLabel", { author: authorName })}
    >
      <div className="flex items-start gap-2">
        <AppAvatar
          size="sm"
          className="mt-0.5 shrink-0"
          label={authorName}
          isBot={!!resolved.author?.is_bot}
          src={resolved.author?.avatar_url}
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs">
            {resolved.author?.is_bot && resolved.author.app_id ? (
              <button
                type="button"
                className="truncate font-semibold text-foreground hover:underline"
                onClick={() => openApp(resolved.author!.app_id!)}
              >
                {authorName}
              </button>
            ) : (
              <span className="truncate font-semibold text-foreground">{authorName}</span>
            )}
            {resolved.author?.is_bot ? (
              <span className="rounded bg-muted px-1 py-0.5 text-3xs font-semibold text-muted-foreground">
                APP
              </span>
            ) : null}
            <time
              className="shrink-0 text-micro text-muted-foreground tabular-nums"
              dateTime={normalizeUtcTimestamp(resolved.created_at)}
              title={fullTimestamp(resolved.created_at, locale)}
            >
              {formatTime(resolved.created_at, locale)}
            </time>
          </div>
          <div className="mt-0.5 flex items-center gap-1 text-xs text-muted-foreground">
            {resolved.is_private && !resolved.is_dm ? (
              <LockKeyholeIcon className="size-3 shrink-0" aria-hidden="true" />
            ) : null}
            {compact ? (
              <span className="truncate">{sourceChannelText}</span>
            ) : (
              <ForwardJumpLink
                channelId={resolved.channel_id}
                messageId={resolved.id}
                parentId={resolved.parent_id}
                className="truncate rounded-sm outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                {sourceChannelText}
              </ForwardJumpLink>
            )}
          </div>
        </div>
      </div>

      <div className="mt-1.5 min-w-0">
        {resolved.content || resolved.card ? (
          <MessageBody
            emojis={resolved.emojis}
            content={resolved.content}
            card={resolved.card}
            components={resolved.components}
            editedAt={resolved.updated_at}
            previews={compact ? "none" : "auto"}
          />
        ) : null}

        {!compact && doc ? (
          <Suspense fallback={null}>
            <ComponentTree document={doc} messageId={resolved.id} channelId={resolved.channel_id} />
          </Suspense>
        ) : null}

        {resolved.updated_at && bodyHidden ? <EditedIndicator at={resolved.updated_at} /> : null}

        {compact ? (
          resolved.attachments.length > 0 ? (
            <p className="mt-1.5 flex items-center gap-1 text-xs text-muted-foreground">
              <PaperclipIcon className="size-3.5 shrink-0" aria-hidden="true" />
              <span>
                {t("messages.forwardAttachmentCount", { count: resolved.attachments.length })}
              </span>
            </p>
          ) : null
        ) : resolved.attachments.length > 0 ? (
          <MessageAttachments attachments={resolved.attachments} />
        ) : null}

        {resolved.has_forwards ? (
          <div className="mt-1.5 flex items-center gap-1 text-xs text-muted-foreground">
            <ForwardIcon className="size-3 shrink-0" aria-hidden="true" />
            {compact ? (
              <span>{t("messages.forwardNestedCompact")}</span>
            ) : (
              <ForwardJumpLink
                channelId={resolved.channel_id}
                messageId={resolved.id}
                parentId={resolved.parent_id}
                className="rounded-sm outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                {t("messages.forwardNestedNote")}
              </ForwardJumpLink>
            )}
          </div>
        ) : null}
      </div>
    </blockquote>
  );
}
