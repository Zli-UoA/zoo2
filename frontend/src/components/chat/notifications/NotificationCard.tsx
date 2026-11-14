import { Link } from "@tanstack/react-router";
import {
  useEffect,
  useMemo,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type RefObject,
  type ComponentProps,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  SmilePlusIcon,
  MessageCircleIcon,
  AtSignIcon,
  UserRoundIcon,
  UsersIcon,
  CheckCheckIcon,
  ArrowUpRightIcon,
  PhoneMissedIcon,
} from "lucide-react";

import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { kitchenLabel, kitchenParts } from "../../../lib/emoji/kitchen";
import { ReactionEmoji } from "../messages/ReactionEmoji";
import type { ChannelRow, ChannelUserRow, UserRow } from "../../../lib/electric/types";
import { INTL_LOCALES } from "../../../lib/i18n/locale";
import { normalizeUtcTimestamp, parseUtcTimestamp } from "../../../lib/timestamps";
import { useLocale } from "../../i18n/LocaleProvider";
import { useShell } from "../../layout/shell-context";
import { formatDmDisplayName, isGroupDm, resolveDmDisplayName } from "../dm";
import { AppAvatar } from "../AppIdentity";
import { MfmContent } from "../messages/MfmContent";
import { fullTimestamp } from "../messages/messageFormat";
import {
  quotePreview,
  notificationReactionGroups,
  type NotificationGroup,
} from "./groupNotifications";
import { useNotificationContext } from "./useNotificationContext";
import { useReadVisibleNotification } from "./useReadVisibleNotification";

export interface NotificationPreviewState {
  visible: boolean | null;
  loading: boolean;
  channelId: string | undefined;
}

export function NotificationCard({
  group,
  channel: providedChannel,
  members = [],
  users,
  onRead,
  scrollRoot,
  readEnabled = true,
  onPreviewChange,
}: {
  group: NotificationGroup;
  channel?: ChannelRow;
  members?: readonly Pick<ChannelUserRow, "user_id">[];
  users: ReadonlyMap<string, UserRow>;
  onRead: (ids: readonly string[]) => void;
  scrollRoot: RefObject<HTMLDivElement | null>;
  readEnabled?: boolean;
  onPreviewChange?: (id: string, state: NotificationPreviewState | null) => void;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const shell = useShell();
  const destinationRef = useRef<HTMLAnchorElement>(null);
  const notification = group.latest;
  const channel = notification.display_data?.channel ?? providedChannel;
  const isReaction = notification.notification_type === "reaction";
  const unreadIds = useMemo(
    () => group.notifications.filter((row) => !row.is_read).map((row) => row.id),
    [group],
  );
  const { ref, hasBeenVisible, isVisible } = useReadVisibleNotification(
    unreadIds,
    onRead,
    scrollRoot,
    readEnabled,
  );
  const context = useNotificationContext(
    channel && hasBeenVisible ? notification.id : null,
    notification.display_data,
  );
  useEffect(() => {
    onPreviewChange?.(notification.id, {
      visible: isVisible,
      loading: context.isLoading,
      channelId: channel?.id,
    });
    return () => onPreviewChange?.(notification.id, null);
  }, [notification.id, isVisible, context.isLoading, channel?.id, onPreviewChange]);
  const message = context.data?.message;
  const avatarReaction = isReaction
    ? notificationReactionGroups(context.data?.reactions ?? [], [group.actorIds[0]!])[0]?.emoji
    : undefined;
  const { catalog } = useKitchenCatalog(!!avatarReaction && kitchenParts(avatarReaction) !== null);
  const name = (id: string) => {
    const user =
      group.notifications.find((row) => row.display_data?.actor?.id === id)?.display_data?.actor ??
      users.get(id);
    return user?.display_name || user?.user_name || id;
  };
  const actor = name(group.actorIds[0]!);
  // 不在着信にはメッセージがない。チャンネルへのリンクだけを出す。
  const isMissedCall = notification.notification_type === "missed_call";
  const label =
    isReaction && group.actorIds.length > 1
      ? t("notifications.reactionGroup", { actor, count: group.actorIds.length - 1 })
      : t(
          isReaction
            ? "notifications.reaction"
            : isMissedCall
              ? "notifications.missedCall"
              : notification.notification_type === "mention"
                ? "notifications.mention"
                : notification.notification_type === "dm"
                  ? "notifications.dm"
                  : "notifications.reply",
          { actor },
        );
  const Icon = isReaction
    ? SmilePlusIcon
    : isMissedCall
      ? PhoneMissedIcon
      : notification.notification_type === "mention"
        ? AtSignIcon
        : MessageCircleIcon;
  const canOpen = Boolean(channel && (message || isMissedCall));

  const channelLabel = channel?.is_dm
    ? formatDmDisplayName(
        resolveDmDisplayName({
          channelName: channel.channel_name,
          members,
          users: [...users.values()],
          currentUserId: notification.user_id,
          locale,
        }),
        t,
      )
    : channel
      ? `#${channel.channel_name}`
      : t("notifications.unavailable");
  const DmIcon = channel && isGroupDm(channel.channel_name) ? UsersIcon : UserRoundIcon;
  const ContextIcon = channel?.is_dm ? DmIcon : Icon;
  const contextLabel = t(
    isReaction
      ? "notifications.reactionContext"
      : isMissedCall
        ? "notifications.missedCallContext"
        : notification.notification_type === "mention"
          ? "notifications.mentionContext"
          : notification.notification_type === "dm"
            ? "notifications.dmContext"
            : "notifications.replyContext",
  );
  const actorSummary =
    group.actorIds.length > 1
      ? t("notifications.actors", { actor, count: group.actorIds.length - 1 })
      : actor;
  function openFromRow(event: ReactMouseEvent<HTMLLIElement>) {
    // The real link remains the keyboard target; text selection and nested controls keep native behavior.
    if (
      !canOpen ||
      event.defaultPrevented ||
      !(event.target instanceof Element) ||
      event.target.closest("a, button, input, textarea, select, summary, [tabindex]") ||
      (window.getSelection()?.toString().length ?? 0) > 0
    )
      return;
    const destination = destinationRef.current;
    if (!destination) return;
    if (event.ctrlKey || event.metaKey || event.shiftKey) {
      window.open(
        destination.href,
        "_blank",
        event.shiftKey ? "popup,noopener,noreferrer" : "noopener,noreferrer",
      );
      return;
    }
    destination.dispatchEvent(
      new MouseEvent("click", {
        bubbles: true,
        cancelable: true,
        view: window,
        button: event.button,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        altKey: event.altKey,
        shiftKey: event.shiftKey,
      }),
    );
  }
  const destination = (
    <p className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
      <ContextIcon className="size-4 shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate font-medium">{channelLabel}</span>
      {canOpen ? (
        <ArrowUpRightIcon className="size-3.5 shrink-0" aria-hidden="true" />
      ) : null}
    </p>
  );
  return (
    <NotificationCardFrame
      ref={ref}
      aria-label={label}
      data-unread={unreadIds.length > 0 || undefined}
      onClick={openFromRow}
      className={canOpen ? "cursor-pointer" : ""}
    >
      <div className="flex items-start gap-3">
        <NotificationAvatar
          badge={
            avatarReaction ? (
              <span
                role="img"
                aria-label={kitchenLabel(avatarReaction)}
                className="flex size-4 items-center justify-center overflow-hidden [&_img]:max-w-full! [&_img]:object-contain"
                data-notification-reaction
              >
                <ReactionEmoji
                  emoji={avatarReaction}
                  catalog={catalog}
                  size={16}
                  emojis={message?.emojis}
                />
              </span>
            ) : (
              <Icon className="size-3" aria-hidden="true" />
            )
          }
        >
          <AppAvatar
            label={actor}
            title={actor}
            isBot={users.get(group.actorIds[0]!)?.is_bot}
            src={users.get(group.actorIds[0]!)?.avatar_url}
          />
        </NotificationAvatar>
        <div className="min-w-0 flex-1 space-y-2 sm:space-y-1">
          <NotificationCardHeader createdAt={notification.created_at} read={!unreadIds.length}>
            <div className="flex min-w-0 flex-1 items-baseline gap-2">
              <p
                className="min-w-0 truncate text-sm font-semibold"
                title={group.actorIds.map(name).join(", ")}
              >
                {actorSummary}
              </p>
              {users.get(group.actorIds[0]!)?.is_bot ? (
                <span className="rounded bg-muted px-1 text-[10px] font-medium text-muted-foreground">
                  APP
                </span>
              ) : null}
            </div>
          </NotificationCardHeader>
          {isMissedCall ? null : channel && message ? (
            <blockquote>
              <div className="mfm-body line-clamp-3 whitespace-pre-wrap break-words text-sm leading-relaxed">
                <MfmContent content={quotePreview(message.content)} emojis={message.emojis} />
              </div>
            </blockquote>
          ) : (
            <p className="text-sm text-muted-foreground">
              {!hasBeenVisible || context.isLoading
                ? t("common.loading")
                : t("notifications.messageUnavailable")}
            </p>
          )}
          {canOpen ? (
            <Link
              ref={destinationRef}
              to="/channels/$channelId"
              params={{ channelId: notification.channel_id }}
              search={{ message: message ? (notification.message_id ?? undefined) : undefined }}
              onClick={() => shell.showChat()}
              title={channelLabel}
              aria-label={`${contextLabel !== channelLabel ? `${contextLabel} · ` : ""}${channelLabel} — ${label}${message ? `: ${quotePreview(message.content)}` : ""}`}
              className="block w-fit max-w-full rounded-sm outline-none hover:underline hover:underline-offset-4 focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              {destination}
            </Link>
          ) : (
            destination
          )}
        </div>
      </div>
    </NotificationCardFrame>
  );
}

/** Shared bordered surface for channel and federated notification cards. */
export function NotificationCardFrame({ className = "", ...props }: ComponentProps<"li">) {
  return (
    <li
      {...props}
      className={`group w-full rounded-lg border p-3 transition-colors data-[unread]:bg-accent/50 hover:bg-muted/40 ${className}`}
    />
  );
}

export function NotificationReadStatus({ read }: { read: boolean }) {
  const { t } = useTranslation();
  return read ? (
    <span
      role="img"
      aria-label={t("notifications.readStatus")}
      className="shrink-0 text-muted-foreground"
    >
      <CheckCheckIcon className="size-4" aria-hidden="true" />
    </span>
  ) : (
    <span
      className="size-2 shrink-0 rounded-full bg-primary"
      role="img"
      aria-label={t("notifications.unreadStatus")}
    />
  );
}

/** Channel avatar dimensions and reaction badge shared by both notification sources. */
export function NotificationAvatar({ children, badge }: { children: ReactNode; badge: ReactNode }) {
  return (
    <div className="relative shrink-0 [&_[data-slot=avatar]]:size-10 sm:[&_[data-slot=avatar]]:size-9">
      {children}
      <span className="absolute -right-1 -bottom-1 flex size-5 items-center justify-center rounded-full bg-muted text-muted-foreground ring-2 ring-background">
        {badge}
      </span>
    </div>
  );
}

export function NotificationTimestamp({ createdAt }: { createdAt: string }) {
  const { locale } = useLocale();
  const formatter = useMemo(
    () =>
      new Intl.DateTimeFormat(INTL_LOCALES[locale], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        hourCycle: "h23",
      }),
    [locale],
  );
  return (
    <time
      className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
      dateTime={normalizeUtcTimestamp(createdAt)}
      title={fullTimestamp(createdAt, locale)}
    >
      {formatter.format(parseUtcTimestamp(createdAt))}
    </time>
  );
}

export function NotificationCardHeader({
  children,
  createdAt,
  read,
}: {
  children: ReactNode;
  createdAt: string;
  read: boolean;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      {children}
      <NotificationTimestamp createdAt={createdAt} />
      <NotificationReadStatus read={read} />
    </div>
  );
}
