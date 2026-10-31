import { useLiveQuery } from "@tanstack/react-db";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelUserRow } from "../../lib/electric/types";
import { useCallback, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { BellIcon } from "lucide-react";
import { toast } from "sonner";

import { useNotifications } from "../../hooks/useNotifications";
import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { NotificationCard } from "./notifications/NotificationCard";
import { groupNotifications } from "./notifications/groupNotifications";

export function NotificationsScreen() {
  const { t } = useTranslation();
  const { notifications, unreadCount, markRead, isLoading, isError } = useNotifications();
  const usersQuery = useUsers();
  const channelsQuery = useActiveChannels();
  const users = useMemo(
    () => new Map((usersQuery.data ?? []).map((row) => [row.id, row])),
    [usersQuery.data],
  );
  const channels = useMemo(
    () => new Map((channelsQuery.data ?? []).map((row) => [row.id, row])),
    [channelsQuery.data],
  );
  const { channelUsers } = useCollections();
  const memberships = useLiveQuery((q) => q.from({ channelUsers }));
  const membersByChannel = useMemo(() => {
    const map = new Map<string, ChannelUserRow[]>();
    for (const member of memberships.data ?? []) {
      const rows = map.get(member.channel_id);
      if (rows) rows.push(member);
      else map.set(member.channel_id, [member]);
    }
    return map;
  }, [memberships.data]);
  const groups = useMemo(() => groupNotifications(notifications), [notifications]);
  const scrollRoot = useRef<HTMLDivElement>(null);
  const attempted = useRef(new Set<string>());
  const readVisible = useCallback(
    (ids: readonly string[]) => {
      const unread = ids.filter((id) => !attempted.current.has(id));
      if (!unread.length) return;
      for (const id of unread) attempted.current.add(id);
      void markRead(unread).catch(() => {
        // Avoid an observer/optimistic-rollback retry loop; revisit the screen to retry.
        toast.error(t("notifications.readError"), { id: "notification-auto-read" });
      });
    },
    [markRead, t],
  );

  return (
    <section className="flex h-full min-h-0 flex-col" aria-labelledby="notifications-title">
      <header className="flex shrink-0 items-center gap-3 border-b px-4 py-4 sm:px-6">
        <span
          className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground"
          aria-hidden="true"
        >
          <BellIcon className="size-5" />
        </span>
        <div>
          <h1 id="notifications-title" className="text-lg font-semibold">
            {t("notifications.title")}
          </h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("notifications.unread", { count: unreadCount })}
          </p>
        </div>
      </header>
      <div ref={scrollRoot} className="min-h-0 flex-1 overflow-y-auto" aria-busy={isLoading}>
        {isError ? (
          <p className="p-6 text-sm text-destructive" role="alert">
            {t("notifications.loadError")}
          </p>
        ) : isLoading ? (
          <p className="p-6 text-sm text-muted-foreground">{t("common.loading")}</p>
        ) : groups.length === 0 ? (
          <div className="flex min-h-72 flex-col items-center justify-center gap-3 p-6 text-center">
            <BellIcon className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm text-muted-foreground">{t("notifications.empty")}</p>
          </div>
        ) : (
          <ul className="w-full" aria-label={t("notifications.title")}>
            {groups.map((group) => (
              <NotificationCard
                key={group.key}
                group={group}
                channel={channels.get(group.latest.channel_id)}
                users={users}
                members={membersByChannel.get(group.latest.channel_id)}
                onRead={readVisible}
                scrollRoot={scrollRoot}
              />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
