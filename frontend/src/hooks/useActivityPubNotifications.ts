import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { useHomeLive } from "./useHomeLive";
export interface ActivityPubNotification {
  id: string;
  actor_id: string;
  display_name: string;
  kind: "follow_request" | "follow_accepted" | "like";
  activity_id: string;
  post_id: string | null;
  content: string | null;
  is_read: boolean;
  created_at: string;
  pending: boolean;
}
export function useActivityPubNotifications() {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [data, setData] = useState<{ items: ActivityPubNotification[]; unread_count: number }>({
    items: [],
    unread_count: 0,
  });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const request = useRef<AbortController | null>(null);
  const refresh = useCallback(async () => {
    request.current?.abort();
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
  const act = async (path: string, body: unknown) => {
    await client.request(path, { method: "POST", body });
    await refresh();
  };
  return { ...data, error, loading, refresh, act };
}
