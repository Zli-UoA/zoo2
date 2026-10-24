import { useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { getCollections } from "../../../lib/electric/collections";

export interface NotificationContext {
  message: { id: string; user_id: string; content: string } | null;
  reactions: { user_id: string; emoji: string }[];
}

/** Fetch a bounded preview, not a channel-wide messages/reactions Shape. */
export function useNotificationContext(notificationId: string | null) {
  const { config } = getCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [state, setState] = useState<{
    id: string | null;
    data: NotificationContext | null;
    isLoading: boolean;
  }>({ id: null, data: null, isLoading: false });
  useEffect(() => {
    if (!notificationId) return;
    let controller: AbortController | null = null;
    const load = () => {
      controller?.abort();
      const request = new AbortController();
      controller = request;
      setState({ id: notificationId, data: null, isLoading: true });
      void client
        .get<NotificationContext>(
          `/api/notifications/${encodeURIComponent(notificationId)}/context`,
          { signal: request.signal },
        )
        .then((data) => {
          if (!request.signal.aborted) setState({ id: notificationId, data, isLoading: false });
        })
        .catch(() => {
          if (!request.signal.aborted)
            setState({ id: notificationId, data: null, isLoading: false });
        });
    };
    load();
    // New notifications change the ID; focus also refreshes edits, removals and current reactions.
    window.addEventListener("focus", load);
    return () => {
      controller?.abort();
      window.removeEventListener("focus", load);
    };
  }, [client, notificationId]);
  if (!notificationId || state.id !== notificationId)
    return { data: null, isLoading: !!notificationId };
  return state;
}
