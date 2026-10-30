import { useCallback, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
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
      <header className="flex items-center justify-between gap-3 border-b px-4 py-3">
        <div>
          <h1 id="notifications-title" className="text-lg font-semibold">
            {t("notifications.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("notifications.unread", { count: unreadCount })}
          </p>
        </div>
      </header>
      <div ref={scrollRoot} className="min-h-0 flex-1 overflow-y-auto p-4" aria-busy={isLoading}>
        {isError ? (
          <p role="alert">{t("notifications.loadError")}</p>
        ) : isLoading ? (
          <p>{t("common.loading")}</p>
        ) : groups.length === 0 ? (
          <p>{t("notifications.empty")}</p>
        ) : (
          <ul className="mx-auto max-w-3xl space-y-3">
            {groups.map((group) => (
              <NotificationCard
                key={group.key}
                group={group}
                channel={channels.get(group.latest.channel_id)}
                users={users}
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
