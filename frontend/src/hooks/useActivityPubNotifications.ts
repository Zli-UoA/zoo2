import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { coalescedRefresh } from "../lib/coalescedRefresh";
import { useHomeLive } from "./useHomeLive";
import { notificationTimestamp } from "../components/chat/notifications/groupNotifications";
export interface ActivityPubNotification {
  id: string;
  actor_id: string;
  display_name: string;
  avatar?: import("../components/home/ActivityPubAvatar").RemoteAvatar | null;
  kind: "follow_request" | "follow_accepted" | "like" | "reply" | "repost";
  activity_id: string;
  post_id: string | null;
  content: string | null;
  content_emojis?: import("../components/home/ActivityPubEmoji").CustomEmoji[];
  is_read: boolean;
  created_at: string;
  pending: boolean;
  actor_emojis?: import("../components/home/ActivityPubEmoji").CustomEmoji[];
  reaction?: string | null;
  emoji?: import("../components/home/ActivityPubEmoji").CustomEmoji | null;
}
type NotificationPage = {
  items: ActivityPubNotification[];
  unread_count: number;
  next_cursor?: string | null;
};
const emptyPage: NotificationPage = { items: [], unread_count: 0 };

export function useActivityPubNotifications() {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [data, setData] = useState<NotificationPage>(emptyPage);
  const snapshot = useRef(data);
  snapshot.current = data;
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState(false);
  const request = useRef<AbortController | null>(null);
  const refreshTask = useRef<Promise<void> | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const olderRequest = useRef<AbortController | null>(null);
  const deferred = useRef(false);
  const pendingReads = useRef(new Set<string>());
  const visibleIds = useRef(new Set<string>());
  const gap = useRef<{ anchors: Set<string>; resume: string | null | undefined } | null>(null);
  const readRevision = useRef(0);
  const refresh = useCallback((): Promise<void> => {
    if (lifetime.current?.signal.aborted) return Promise.resolve();
    if (olderRequest.current) {
      deferred.current = true;
      return Promise.resolve();
    }
    if (refreshTask.current) {
      deferred.current = true;
      return refreshTask.current;
    }
    const controller = new AbortController();
    request.current = controller;
    const task = (async () => {
      do {
        deferred.current = false;
        const readsAtStart = readRevision.current;
        try {
          const page = await client.get<NotificationPage>("/api/activitypub/notifications", {
            signal: controller.signal,
          });
          const headIds = new Set(page.items.map((item) => item.id));
          const checkedIds = [...visibleIds.current].filter((id) => !headIds.has(id)).slice(0, 100);
          let checked: ActivityPubNotification[] = [];
          if (checkedIds.length) {
            const viewport = await client.request<NotificationPage>(
              "/api/activitypub/notifications/entries",
              {
                method: "POST",
                body: { entries: checkedIds },
                signal: controller.signal,
              },
            );
            checked = viewport.items;
          }
          if (controller.signal.aborted) return;
          const updates = new Map([...page.items, ...checked].map((item) => [item.id, item]));
          const retained = snapshot.current.items;
          const latest = new Map(retained.map((item) => [item.id, item]));
          const authoritative = new Set(checkedIds);
          const items = [
            ...page.items,
            ...(page.next_cursor ? retained.filter((item) => !headIds.has(item.id)) : []),
          ]
            .filter((item) => !authoritative.has(item.id) || updates.has(item.id))
            .map((item) => {
              const updated = updates.get(item.id) ?? item;
              return latest.get(item.id)?.is_read ? { ...updated, is_read: true } : updated;
            })
            .sort(
              (a, b) =>
                notificationTimestamp(b.created_at) - notificationTimestamp(a.created_at) ||
                (a.id > b.id ? -1 : a.id < b.id ? 1 : 0),
            );
          let cursor = page.next_cursor;
          if (page.next_cursor && retained.length) {
            if (retained.some((item) => headIds.has(item.id))) {
              if (
                gap.current ||
                snapshot.current.next_cursor == null ||
                retained.some((item) => !headIds.has(item.id))
              )
                cursor = snapshot.current.next_cursor;
            } else {
              gap.current ??= {
                anchors: new Set(retained.map((item) => item.id)),
                resume: snapshot.current.next_cursor,
              };
            }
          } else gap.current = null;
          if (!controller.signal.aborted) {
            const updated = {
              ...page,
              next_cursor: cursor,
              items: items.map((item) =>
                pendingReads.current.has(item.id) ? { ...item, is_read: true } : item,
              ),
              unread_count:
                readsAtStart !== readRevision.current
                  ? snapshot.current.unread_count
                  : Math.max(
                      0,
                      page.unread_count -
                        items.filter((item) => !item.is_read && pendingReads.current.has(item.id))
                          .length,
                    ),
            };
            snapshot.current = updated;
            setData(updated);
            setError(false);
          }
        } catch {
          if (!controller.signal.aborted) setError(true);
        } finally {
          if (!controller.signal.aborted) setLoading(false);
        }
      } while (
        deferred.current &&
        !controller.signal.aborted &&
        document.visibilityState !== "hidden"
      );
    })().finally(() => {
      if (request.current === controller) {
        request.current = null;
        refreshTask.current = null;
      }
    });
    refreshTask.current = task;
    return task;
  }, [client]);
  const schedule = useRef<() => void>(() => {});
  useEffect(() => {
    lifetime.current = new AbortController();
    const scheduler = coalescedRefresh(() =>
      document.visibilityState === "hidden" ? undefined : refresh(),
    );
    schedule.current = scheduler.request;
    scheduler.request();
    return () => {
      schedule.current = () => {};
      scheduler.dispose();
      lifetime.current?.abort();
      request.current?.abort();
      olderRequest.current?.abort();
      request.current = null;
      refreshTask.current = null;
      olderRequest.current = null;
      deferred.current = false;
      gap.current = null;
    };
  }, [refresh]);
  useHomeLive(() => schedule.current());
  const loadMore = async () => {
    const scope = lifetime.current;
    if (!scope || scope.signal.aborted) return;
    // Finish the head refresh before reading its cursor; never cancel it to load older rows.
    await refreshTask.current;
    const cursor = snapshot.current.next_cursor;
    if (scope.signal.aborted || !cursor || olderRequest.current) return;
    setLoadingMore(true);
    const controller = new AbortController();
    olderRequest.current = controller;
    try {
      const next = await client.get<NotificationPage>(
        `/api/activitypub/notifications?before=${encodeURIComponent(cursor)}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        const current = snapshot.current;
        const updated = {
          ...next,
          next_cursor: next.next_cursor,
          items: [
            ...current.items,
            ...next.items.filter((item) => !current.items.some((old) => old.id === item.id)),
          ].sort(
            (a, b) =>
              notificationTimestamp(b.created_at) - notificationTimestamp(a.created_at) ||
              (a.id > b.id ? -1 : a.id < b.id ? 1 : 0),
          ),
        };
        if (gap.current && next.items.some((item) => gap.current?.anchors.has(item.id))) {
          updated.next_cursor = gap.current.resume;
          gap.current = null;
        } else if (!next.next_cursor) gap.current = null;
        snapshot.current = updated;
        setData(updated);
        setError(false);
      }
    } catch {
      if (!controller.signal.aborted) setError(true);
    } finally {
      if (olderRequest.current === controller) {
        olderRequest.current = null;
        setLoadingMore(false);
        if (deferred.current) void refresh();
      }
    }
  };
  const markRead = useCallback(
    async (ids?: readonly string[]) => {
      const selected =
        ids === undefined
          ? snapshot.current.items.filter((item) => !item.is_read).map((item) => item.id)
          : [...new Set(ids)];
      if (ids !== undefined && !selected.length) return;
      for (const id of selected) pendingReads.current.add(id);
      const previous = snapshot.current;
      readRevision.current++;
      const changed = previous.items
        .filter((item) => !item.is_read && selected.includes(item.id))
        .map((item) => item.id);
      const updated = {
        ...previous,
        items: previous.items.map((item) =>
          selected.includes(item.id) ? { ...item, is_read: true } : item,
        ),
        unread_count: ids === undefined ? 0 : Math.max(0, previous.unread_count - changed.length),
      };
      snapshot.current = updated;
      setData(updated);
      try {
        await client.request("/api/activitypub/notifications/read", {
          method: "POST",
          body: ids === undefined ? {} : { ids: selected },
        });
      } catch (error) {
        readRevision.current++;
        selected.forEach((id) => pendingReads.current.delete(id));
        const current = snapshot.current;
        const restored = {
          ...current,
          items: current.items.map((item) =>
            changed.includes(item.id) ? { ...item, is_read: false } : item,
          ),
          unread_count:
            current.unread_count + (ids === undefined ? previous.unread_count : changed.length),
        };
        snapshot.current = restored;
        setData(restored);
        await refresh();
        throw error;
      } finally {
        selected.forEach((id) => pendingReads.current.delete(id));
      }
    },
    [client, refresh],
  );
  const act = async (path: string, body: unknown) => {
    await client.request(path, { method: "POST", body });
    await refresh();
  };
  const setVisible = useCallback((id: string, visible: boolean) => {
    const wasVisible = visibleIds.current.has(id);
    if (visible) visibleIds.current.add(id);
    else visibleIds.current.delete(id);
    if (visible && !wasVisible && snapshot.current.items.findIndex((item) => item.id === id) >= 100)
      schedule.current();
  }, []);
  return { ...data, error, loading, loadingMore, loadMore, refresh, act, markRead, setVisible };
}
