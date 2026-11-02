import { useUnifiedNotifications } from "../../hooks/useUnifiedNotifications";
import { ActivityPubNotificationCard } from "../home/ActivityPubNotifications";
import { Button } from "../ui/button";
import { useLiveQuery } from "@tanstack/react-db";
import { useCollections } from "../../lib/electric/collections-context";
import type { ChannelUserRow } from "../../lib/electric/types";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BellIcon } from "lucide-react";
import { toast } from "sonner";

import { useActiveChannels, useUsers } from "../../hooks/useRealtime";
import { Spinner } from "../ui/spinner";
import { ScreenHeader } from "../layout/PaneHeader";
import { NotificationCard, type NotificationPreviewState } from "./notifications/NotificationCard";

export function NotificationsScreen() {
  const { t } = useTranslation();
  const { channel, activitypub: apub, entries, markRead, unreadCount } = useUnifiedNotifications();
  const { isLoading, isError } = channel;
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
  const groups = useMemo(
    () => entries.flatMap((entry) => (entry.source === "channel" ? [entry.group] : [])),
    [entries],
  );
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
    apub.loading ||
    (!isError && (isLoading || usersQuery.isLoading || channelsQuery.isLoading || previewsLoading));
  const [hasLoaded, setHasLoaded] = useState(false);
  useEffect(() => {
    if (!loading && !isError) setHasLoaded(true);
  }, [loading, isError]);
  const initialLoading = loading && !hasLoaded;
  const scrollRoot = useRef<HTMLDivElement>(null);
  const attempted = useRef(new Set<string>());
  const queued = useRef(new Set<string>());
  const flush = useRef<ReturnType<typeof setTimeout> | null>(null);
  const writeRead = useRef(markRead);
  writeRead.current = markRead;
  useEffect(
    () => () => {
      if (flush.current) clearTimeout(flush.current);
      queued.current.clear();
    },
    [],
  );
  const readVisible = useCallback(
    (ids: readonly string[]) => {
      for (const id of ids)
        if (!attempted.current.has(id)) {
          attempted.current.add(id);
          queued.current.add(id);
        }
      if (!queued.current.size || flush.current) return;
      flush.current = setTimeout(() => {
        const selected = [...queued.current];
        queued.current.clear();
        flush.current = null;
        void writeRead
          .current(selected)
          .catch(() => toast.error(t("notifications.readError"), { id: "notification-auto-read" }));
      }, 0);
    },
    [t],
  );
  const [reading, setReading] = useState(false);
  const readAll = async () => {
    if (reading) return;
    setReading(true);
    try {
      await markRead();
    } catch {
      toast.error(t("notifications.readError"));
    } finally {
      setReading(false);
    }
  };

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
        actions={
          unreadCount > 0 ? (
            <Button variant="ghost" size="sm" disabled={reading} onClick={() => void readAll()}>
              {t("home.readAll")}
            </Button>
          ) : undefined
        }
      />
      <div ref={scrollRoot} className="min-h-0 flex-1 overflow-y-auto" aria-busy={loading}>
        {apub.error ? (
          <p role="alert" className="p-4 text-sm text-destructive">
            {t("home.controlsError")}{" "}
            <Button variant="link" onClick={() => void apub.refresh()}>
              {t("home.refresh")}
            </Button>
          </p>
        ) : null}
        {isError ? (
          <p className="p-6 text-sm text-destructive" role="alert">
            {t("notifications.loadError")}
          </p>
        ) : null}
        {!isLoading &&
        !apub.loading &&
        !apub.error &&
        !apub.items.length &&
        entries.length === 0 &&
        !isError ? (
          <div className="flex min-h-72 flex-col items-center justify-center gap-3 p-6 text-center">
            <BellIcon className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm text-muted-foreground">{t("notifications.empty")}</p>
          </div>
        ) : (
          <ul
            className={`w-full space-y-3 p-4 ${initialLoading ? "opacity-0" : ""}`}
            aria-label={t("notifications.title")}
            aria-hidden={initialLoading || undefined}
            inert={initialLoading}
          >
            {entries.map((entry) =>
              entry.source === "activitypub" ? (
                <ActivityPubNotificationCard
                  key={entry.key}
                  item={entry.item}
                  onRead={readVisible}
                  scrollRoot={scrollRoot}
                  readEnabled={!loading}
                  act={apub.act}
                />
              ) : (
                <NotificationCard
                  key={entry.key}
                  group={entry.group}
                  channel={channels.get(entry.group.latest.channel_id)}
                  users={users}
                  members={membersByChannel.get(entry.group.latest.channel_id)}
                  onRead={readVisible}
                  scrollRoot={scrollRoot}
                  readEnabled={!loading}
                  onPreviewChange={previewChanged}
                />
              ),
            )}
          </ul>
        )}
        {apub.next_cursor ? (
          <div className="px-4 pb-4">
            <Button
              variant="outline"
              disabled={apub.loadingMore}
              onClick={() => void apub.loadMore()}
            >
              {t(apub.loadingMore ? "home.loading" : "home.moreNotifications")}
            </Button>
          </div>
        ) : null}
        {loading && !initialLoading ? (
          <Spinner label={t("common.loading")} className="sticky bottom-0 bg-background py-4" />
        ) : null}
      </div>
      {initialLoading ? (
        <Spinner label={t("common.loading")} className="absolute inset-0 z-20 bg-background" />
      ) : null}
    </section>
  );
}
