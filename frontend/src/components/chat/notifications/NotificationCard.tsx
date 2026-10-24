import { eq, useLiveQuery } from "@tanstack/react-db";
import { Link } from "@tanstack/react-router";
import { useMemo, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { HeartIcon, MessageCircleIcon, AtSignIcon } from "lucide-react";

import { useChannelAccess } from "../../../hooks/useRealtime";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { getCollections } from "../../../lib/electric/collections";
import type { ChannelRow, UserRow } from "../../../lib/electric/types";
import { groupReactions } from "../../../lib/emoji/reactions";
import { kitchenLabel, kitchenParts } from "../../../lib/emoji/kitchen";
import { useLocale } from "../../i18n/LocaleProvider";
import { useShell } from "../../layout/shell-context";
import { Avatar, AvatarFallback, AvatarGroup, AvatarGroupCount } from "../../ui/avatar";
import { initials } from "../format";
import { ReactionEmoji } from "../messages/ReactionEmoji";
import { fullTimestamp } from "../messages/messageFormat";
import { quotePreview, type NotificationGroup } from "./groupNotifications";
import { useReadVisibleNotification } from "./useReadVisibleNotification";

export function NotificationCard({
  group,
  channel,
  users,
  onRead,
  scrollRoot,
}: {
  group: NotificationGroup;
  channel?: ChannelRow;
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
  const ref = useReadVisibleNotification(unreadIds, onRead, scrollRoot);
  const collections = getCollections();
  const access = useChannelAccess(channel ?? null);
  const readable = channel && access.canRead && !access.isMembershipPending;
  const messages = readable ? collections.messages(channel.id) : null;
  const reactions = readable && isReaction ? collections.reactions(channel.id) : null;
  const messageQuery = useLiveQuery(
    (q) =>
      messages
        ? q.from({ messages }).where(({ messages: row }) => eq(row.id, notification.message_id))
        : undefined,
    [messages, notification.message_id],
  );
  const reactionQuery = useLiveQuery(
    (q) =>
      reactions
        ? q
            .from({ reactions })
            .where(({ reactions: row }) => eq(row.message_id, notification.message_id))
        : undefined,
    [reactions, notification.message_id],
  );
  const message = messageQuery.data?.[0];
  const actorIds = new Set(group.actorIds);
  const reactionGroups =
    groupReactions(
      (reactionQuery.data ?? []).filter((row) => actorIds.has(row.user_id)),
      null,
    ).get(notification.message_id) ?? [];
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
              : "notifications.reply",
          { actor },
        );
  const Icon = isReaction
    ? HeartIcon
    : notification.notification_type === "mention"
      ? AtSignIcon
      : MessageCircleIcon;

  return (
    <li
      ref={ref}
      className={`rounded-xl border p-4 ${unreadIds.length ? "border-brand/40 bg-brand-muted/20" : "bg-card"}`}
    >
      <div className="flex items-start gap-3">
        <span className="mt-1 text-brand">
          <Icon className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <AvatarGroup>
              {group.actorIds.slice(0, 8).map((id) => (
                <Avatar key={id} size="sm" title={name(id)}>
                  <AvatarFallback>{initials(name(id))}</AvatarFallback>
                </Avatar>
              ))}
              {group.actorIds.length > 8 ? (
                <AvatarGroupCount>+{group.actorIds.length - 8}</AvatarGroupCount>
              ) : null}
            </AvatarGroup>
            <time className="text-xs text-muted-foreground" dateTime={notification.created_at}>
              {fullTimestamp(notification.created_at, locale)}
            </time>
          </div>
          <p className="text-sm font-medium">{label}</p>
          <p className="text-xs text-muted-foreground">
            {channel ? `#${channel.channel_name}` : t("notifications.unavailable")}
          </p>
          {reactionGroups.length ? (
            <ul className="flex flex-wrap gap-1.5" aria-label={t("notifications.reactions")}>
              {reactionGroups.map((reaction) => (
                <li
                  key={reaction.emoji}
                  className="flex items-center gap-1 rounded-full border bg-muted/50 px-2 py-0.5 text-sm"
                  aria-label={`${kitchenLabel(reaction.emoji)} ${reaction.count}`}
                  title={kitchenLabel(reaction.emoji)}
                >
                  <ReactionEmoji emoji={reaction.emoji} catalog={catalog} size={20} />
                  <span className="tabular-nums text-muted-foreground">{reaction.count}</span>
                </li>
              ))}
            </ul>
          ) : null}
          {readable && message && !message.deleted_at ? (
            <Link
              to="/channels/$channelId"
              params={{ channelId: notification.channel_id }}
              search={{ message: notification.message_id }}
              onClick={() => shell.showChat()}
              className="block rounded-md border-l-2 border-brand/40 bg-muted/40 px-3 py-2 transition-colors outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <blockquote>
                <p className="mb-1 truncate text-xs font-medium text-muted-foreground">
                  {name(message.user_id)}
                </p>
                <p className="line-clamp-3 whitespace-pre-wrap break-words text-sm leading-relaxed">
                  {quotePreview(message.content)}
                </p>
              </blockquote>
            </Link>
          ) : (
            <p className="rounded-md border-l-2 px-3 py-2 text-sm text-muted-foreground">
              {messageQuery.isLoading || access.isMembershipPending
                ? t("common.loading")
                : t("notifications.messageUnavailable")}
            </p>
          )}
        </div>
      </div>
    </li>
  );
}
