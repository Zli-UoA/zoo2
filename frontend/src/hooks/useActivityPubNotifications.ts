import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
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
  const olderRequest = useRef<AbortController | null>(null);
  const deferred = useRef(false);
  const pendingReads = useRef(new Set<string>());
  const refresh = useCallback(async () => {
    if (olderRequest.current) {
      deferred.current = true;
      return;
    }
    deferred.current = false;
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const retained = snapshot.current.items;
    const tail = retained.at(-1);
    try {
      let page: NotificationPage;
      const items: ActivityPubNotification[] = [];
      let before: string | null = null;
      const visited = new Set<string>();
      do {
        page = await client.get<NotificationPage>(
          `/api/activitypub/notifications${before ? `?before=${encodeURIComponent(before)}` : ""}`,
          { signal: controller.signal },
        );
        items.push(...page.items.filter((item) => !items.some((old) => old.id === item.id)));
        before = page.next_cursor ?? null;
        const last = items.at(-1);
        const reached =
          !tail ||
          items.some((item) => item.id === tail.id) ||
          (!!last &&
            (notificationTimestamp(last.created_at) < notificationTimestamp(tail.created_at) ||
              (notificationTimestamp(last.created_at) === notificationTimestamp(tail.created_at) &&
                last.id <= tail.id)));
        if (!before || visited.has(before) || (reached && items.length >= retained.length)) break;
        visited.add(before);
      } while (before && !controller.signal.aborted);
      if (!controller.signal.aborted) {
        const updated = {
          ...page,
          items: items.map((item) =>
            pendingReads.current.has(item.id) ? { ...item, is_read: true } : item,
          ),
          unread_count: Math.max(
            0,
            page.unread_count -
              items.filter((item) => !item.is_read && pendingReads.current.has(item.id)).length,
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
  }, [client]);
  useEffect(() => {
    void refresh();
    return () => {
      request.current?.abort();
      olderRequest.current?.abort();
      deferred.current = false;
    };
  }, [refresh]);
  useHomeLive(() => void refresh());
  const loadMore = async () => {
    if (!data.next_cursor || olderRequest.current || loading) return;
    setLoadingMore(true);
    request.current?.abort();
    const controller = new AbortController();
    olderRequest.current = controller;
    try {
      const next = await client.get<NotificationPage>(
        `/api/activitypub/notifications?before=${encodeURIComponent(data.next_cursor)}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        const current = snapshot.current;
        const updated = {
          ...next,
          items: [
            ...current.items,
            ...next.items.filter((item) => !current.items.some((old) => old.id === item.id)),
          ],
        };
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
  return { ...data, error, loading, loadingMore, loadMore, refresh, act, markRead };
}
