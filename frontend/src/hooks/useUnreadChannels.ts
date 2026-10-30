import { useEffect, useMemo, useState } from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

export interface ChannelReadState {
  channel_id: string;
  last_read_at: string | null;
  has_unread: boolean;
}

export const READ_STATE_CHANGED = "zoo-read-state-changed";

/** Refresh inactive channels without opening a message subscription for every channel. */
export function useUnreadChannels(userId: string | null): Set<string> {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config, userId]);
  const [result, setResult] = useState<{ userId: string; ids: Set<string> } | null>(null);
  useEffect(() => {
    if (!userId) return;
    let active = true;
    let inFlight = false;
    const refresh = async () => {
      if (inFlight || document.visibilityState === "hidden") return;
      inFlight = true;
      try {
        const states = await client.get<ChannelReadState[]>("/api/channel-read-states");
        if (active)
          setResult({
            userId,
            ids: new Set(states.filter((s) => s.has_unread).map((s) => s.channel_id)),
          });
      } catch {
        // Preserve the last successful result and retry on the next refresh.
      } finally {
        inFlight = false;
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 10000);
    window.addEventListener("focus", refresh);
    window.addEventListener(READ_STATE_CHANGED, refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      active = false;
      window.clearInterval(interval);
      window.removeEventListener("focus", refresh);
      window.removeEventListener(READ_STATE_CHANGED, refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [client, userId]);
  return result?.userId === userId ? result.ids : new Set();
}
