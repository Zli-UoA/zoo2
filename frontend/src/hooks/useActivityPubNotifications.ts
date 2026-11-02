import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { useHomeLive } from "./useHomeLive";
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
export function useActivityPubNotifications() {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [data, setData] = useState<{
    items: ActivityPubNotification[];
    unread_count: number;
    next_cursor?: string | null;
  }>({
    items: [],
    unread_count: 0,
  });
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState(false);
  const request = useRef<AbortController | null>(null);
  const refresh = useCallback(async () => {
    request.current?.abort();
    setLoadingMore(false);
    const controller = new AbortController();
    request.current = controller;
    try {
      const next = await client.get<typeof data>("/api/activitypub/notifications", {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        setData(next);
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
    return () => request.current?.abort();
  }, [refresh]);
  useHomeLive(() => void refresh());
  const loadMore = async () => {
    if (!data.next_cursor || loadingMore || loading) return;
    setLoadingMore(true);
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    try {
      const next = await client.get<typeof data>(
        `/api/activitypub/notifications?before=${encodeURIComponent(data.next_cursor)}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        setData((current) => ({
          ...next,
          items: [
            ...current.items,
            ...next.items.filter((item) => !current.items.some((old) => old.id === item.id)),
          ],
        }));
        setError(false);
      }
    } catch {
      if (!controller.signal.aborted) setError(true);
    } finally {
      if (!controller.signal.aborted) setLoadingMore(false);
    }
  };
  const act = async (path: string, body: unknown) => {
    await client.request(path, { method: "POST", body });
    await refresh();
  };
  return { ...data, error, loading, loadingMore, loadMore, refresh, act };
}
