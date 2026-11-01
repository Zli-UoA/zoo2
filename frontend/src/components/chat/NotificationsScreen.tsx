import { useActivityPubNotifications } from "../../hooks/useActivityPubNotifications";
import { ActivityPubNotifications } from "../home/ActivityPubNotifications";
import { useLiveQuery } from "@tanstack/react-db";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelUserRow } from "../../lib/electric/types";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BellIcon } from "lucide-react";
import { toast } from "sonner";

import { useNotifications } from "../../hooks/useNotifications";
import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { Spinner } from "../ui/spinner";
import { ScreenHeader } from "../layout/PaneHeader";
import { NotificationCard, type NotificationPreviewState } from "./notifications/NotificationCard";
import { groupNotifications } from "./notifications/groupNotifications";

export function NotificationsScreen() {
  const { t } = useTranslation();
  const apub = useActivityPubNotifications();
  const { notifications, markRead, isLoading, isError } = useNotifications();
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
  const [previews, setPreviews] = useState(() => new Map<string, NotificationPreviewState>());
  const previewChanged = useCallback((id: string, state: NotificationPreviewState | null) => {
    setPreviews((current) => {
      const previous = current.get(id);
      if (!state && !previous) return current;
      if (
        state &&
        previous?.visible === state.visible &&
        previous.loading === state.loading &&
        previous.channelId === state.channelId
      )
        return current;
      const next = new Map(current);
      if (state) next.set(id, state);
      else next.delete(id);
      return next;
    });
  }, []);
  const previewsLoading = groups.some((group) => {
    const state = previews.get(group.latest.id);
    if (!state || state.visible === null) return true;
    return (
      state.visible &&
      (state.loading ||
        (channels.has(group.latest.channel_id) && state.channelId !== group.latest.channel_id))
    );
  });
  const loading =
    !isError && (isLoading || usersQuery.isLoading || channelsQuery.isLoading || previewsLoading);
  const [hasLoaded, setHasLoaded] = useState(false);
  useEffect(() => {
    if (!loading && !isError) setHasLoaded(true);
  }, [loading, isError]);
  const initialLoading = loading && !hasLoaded;
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
    <section
      className="relative flex h-full min-h-0 flex-col"
      aria-labelledby="notifications-title"
      aria-busy={loading}
    >
      <ScreenHeader
        icon={BellIcon}
        iconClassName="bg-primary text-primary-foreground"
        titleId="notifications-title"
        title={t("notifications.title")}
      />
      <div ref={scrollRoot} className="min-h-0 flex-1 overflow-y-auto" aria-busy={loading}>
        <ActivityPubNotifications notifications={apub} />
        {isError ? (
          <p className="p-6 text-sm text-destructive" role="alert">
            {t("notifications.loadError")}
          </p>
        ) : !isLoading &&
          !apub.loading &&
          !apub.error &&
          !apub.items.length &&
          groups.length === 0 ? (
          <div className="flex min-h-72 flex-col items-center justify-center gap-3 p-6 text-center">
            <BellIcon className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm text-muted-foreground">{t("notifications.empty")}</p>
          </div>
        ) : (
          <ul
            className={`w-full ${initialLoading ? "opacity-0" : ""}`}
            aria-label={t("notifications.title")}
            aria-hidden={initialLoading || undefined}
            inert={initialLoading}
          >
            {groups.map((group) => (
              <NotificationCard
                key={group.key}
                group={group}
                channel={channels.get(group.latest.channel_id)}
                users={users}
                members={membersByChannel.get(group.latest.channel_id)}
                onRead={readVisible}
                scrollRoot={scrollRoot}
                readEnabled={!loading}
                onPreviewChange={previewChanged}
              />
            ))}
          </ul>
        )}
        {loading && !initialLoading ? (
          <Spinner label={t("common.loading")} className="sticky bottom-0 bg-background py-4" />
        ) : null}
      </div>
      {initialLoading && !apub.items.length ? (
        <Spinner label={t("common.loading")} className="absolute inset-0 z-20 bg-background" />
      ) : null}
    </section>
  );
}
