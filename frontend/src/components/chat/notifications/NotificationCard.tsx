import { Link } from "@tanstack/react-router";
import { useMemo, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import {
  HeartIcon,
  MessageCircleIcon,
  AtSignIcon,
  UserRoundIcon,
  UsersIcon,
  CheckCheckIcon,
  ArrowUpRightIcon,
} from "lucide-react";

import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import type { ChannelRow, ChannelUserRow, UserRow } from "../../../lib/electric/types";
import { kitchenLabel, kitchenParts } from "../../../lib/emoji/kitchen";
import { INTL_LOCALES } from "../../../lib/i18n/locale";
import { normalizeUtcTimestamp, parseUtcTimestamp } from "../../../lib/timestamps";
import { useLocale } from "../../i18n/LocaleProvider";
import { useShell } from "../../layout/shell-context";
import { formatDmDisplayName, isGroupDm, resolveDmDisplayName } from "../dm";
import { AppAvatar } from "../AppIdentity";
import { MfmContent } from "../messages/MfmContent";
import { ReactionEmoji } from "../messages/ReactionEmoji";
import { fullTimestamp } from "../messages/messageFormat";
import {
  quotePreview,
  notificationReactionGroups,
  type NotificationGroup,
} from "./groupNotifications";
import { useNotificationContext } from "./useNotificationContext";
import { useReadVisibleNotification } from "./useReadVisibleNotification";

export function NotificationCard({
  group,
  channel,
  members = [],
  users,
  onRead,
  scrollRoot,
}: {
  group: NotificationGroup;
  channel?: ChannelRow;
  members?: readonly Pick<ChannelUserRow, "user_id">[];
  users: ReadonlyMap<string, UserRow>;
  onRead: (ids: readonly string[]) => void;
  scrollRoot: RefObject<HTMLDivElement | null>;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const shell = useShell();
  const notification = group.latest;
  const isReaction = notification.notification_type === "reaction";
  const unreadIds = useMemo(
    () => group.notifications.filter((row) => !row.is_read).map((row) => row.id),
    [group],
  );
  const { ref, hasBeenVisible } = useReadVisibleNotification(unreadIds, onRead, scrollRoot);
  const context = useNotificationContext(channel && hasBeenVisible ? notification.id : null);
  const message = context.data?.message;
  const reactionGroups = isReaction
    ? notificationReactionGroups(context.data?.reactions ?? [], group.actorIds)
    : [];
  const { catalog } = useKitchenCatalog(
    reactionGroups.some((reaction) => kitchenParts(reaction.emoji) !== null),
  );
  const name = (id: string) => {
    const user = users.get(id);
    return user?.display_name || user?.user_name || id;
  };
  const actor = name(group.actorIds[0]!);
  const label =
    isReaction && group.actorIds.length > 1
      ? t("notifications.reactionGroup", { actor, count: group.actorIds.length - 1 })
      : t(
          isReaction
            ? "notifications.reaction"
            : notification.notification_type === "mention"
              ? "notifications.mention"
              : notification.notification_type === "dm"
                ? "notifications.dm"
                : "notifications.reply",
          { actor },
        );
  const Icon = isReaction
    ? HeartIcon
    : notification.notification_type === "mention"
      ? AtSignIcon
      : MessageCircleIcon;

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
  const dateFormatter = useMemo(
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
  const destination = (
    <p className="flex min-w-0 items-center gap-1.5 text-sm text-muted-foreground">
      <ContextIcon className="size-4 shrink-0" aria-hidden="true" />
      {contextLabel !== channelLabel ? (
        <>
          <span className="min-w-0 truncate">{contextLabel}</span>
          <span aria-hidden="true">·</span>
        </>
      ) : null}
      <span className="min-w-0 truncate font-medium">{channelLabel}</span>
      {channel && message ? (
        <ArrowUpRightIcon className="size-3.5 shrink-0" aria-hidden="true" />
      ) : null}
    </p>
  );
  return (
    <li
      ref={ref}
      aria-label={label}
      data-unread={unreadIds.length > 0 || undefined}
      className="group relative isolate w-full border-b px-4 py-5 transition-colors hover:bg-muted/40 sm:px-6 sm:py-6"
    >
      <div className="flex items-start gap-3 sm:gap-4">
        <div className="relative shrink-0">
          <AppAvatar
            label={actor}
            title={actor}
            isBot={users.get(group.actorIds[0]!)?.is_bot}
            src={users.get(group.actorIds[0]!)?.avatar_url}
            className="size-10 sm:size-11"
          />
          <span className="absolute -right-1 -bottom-1 flex size-5 items-center justify-center rounded-full bg-muted text-muted-foreground ring-2 ring-background">
            <Icon className="size-3" aria-hidden="true" />
          </span>
        </div>
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex min-w-0 items-center gap-2">
            <div className="flex min-w-0 flex-1 items-baseline gap-2">
              <p
                className="relative z-10 min-w-0 truncate text-sm font-semibold sm:text-base"
                title={group.actorIds.map(name).join(", ")}
              >
                {actorSummary}
              </p>
              {users.get(group.actorIds[0]!)?.is_bot ? (
                <span className="rounded bg-muted px-1 text-[10px] font-medium text-muted-foreground">
                  APP
                </span>
              ) : null}
              <time
                className="shrink-0 text-xs text-muted-foreground sm:text-sm"
                dateTime={normalizeUtcTimestamp(notification.created_at)}
                title={fullTimestamp(notification.created_at, locale)}
              >
                {dateFormatter.format(parseUtcTimestamp(notification.created_at))}
              </time>
            </div>
            <span className="flex w-14 shrink-0 justify-end">
              {unreadIds.length ? (
                <span className="rounded-full bg-primary px-2 py-0.5 text-[10px] font-semibold text-primary-foreground">
                  {t("notifications.unreadStatus")}
                </span>
              ) : (
                <span
                  role="img"
                  aria-label={t("notifications.readStatus")}
                  title={t("notifications.readStatus")}
                  className="relative z-10 text-muted-foreground"
                >
                  <CheckCheckIcon className="size-4" aria-hidden="true" />
                </span>
              )}
            </span>
          </div>
          {channel && message ? (
            <blockquote>
              <div className="mfm-body pointer-events-none relative z-10 line-clamp-3 whitespace-pre-wrap break-words text-sm leading-relaxed sm:text-base [&_a]:pointer-events-auto [&_button]:pointer-events-auto [&_input]:pointer-events-auto">
                <MfmContent content={quotePreview(message.content)} />
              </div>
            </blockquote>
          ) : (
            <p className="text-sm text-muted-foreground">
              {!hasBeenVisible || context.isLoading
                ? t("common.loading")
                : t("notifications.messageUnavailable")}
            </p>
          )}
          {channel && message ? (
            <Link
              to="/channels/$channelId"
              params={{ channelId: notification.channel_id }}
              search={{ message: notification.message_id }}
              onClick={() => shell.showChat()}
              title={channelLabel}
              aria-label={`${contextLabel !== channelLabel ? `${contextLabel} · ` : ""}${channelLabel} — ${label}: ${quotePreview(message.content)}`}
              className="block w-fit max-w-full outline-none after:absolute after:inset-0 after:z-0 hover:underline hover:underline-offset-4 focus-visible:after:ring-2 focus-visible:after:ring-inset focus-visible:after:ring-ring"
            >
              {destination}
            </Link>
          ) : (
            destination
          )}
          {reactionGroups.length ? (
            <ul className="flex flex-wrap gap-1.5 pt-0.5" aria-label={t("notifications.reactions")}>
              {reactionGroups.map((reaction) => (
                <li
                  key={reaction.emoji}
                  className="flex items-center gap-1 rounded-full bg-muted px-2.5 py-1 text-sm"
                  aria-label={`${kitchenLabel(reaction.emoji)} ${reaction.count}`}
                  title={kitchenLabel(reaction.emoji)}
                >
                  <ReactionEmoji emoji={reaction.emoji} catalog={catalog} size={20} />
                  <span className="tabular-nums text-muted-foreground">{reaction.count}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </div>
    </li>
  );
}
