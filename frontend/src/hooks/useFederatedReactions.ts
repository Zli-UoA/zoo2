import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import type { MessageReactionRow } from "../lib/electric/types";
import { coalescedRefresh } from "../lib/coalescedRefresh";
import { useHomeLive } from "./useHomeLive";

export type FederatedReactionRow = MessageReactionRow & { display_name: string; user_name: string };

export function useFederatedReactions(channelId: string) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [state, setState] = useState<{
    channel: string;
    rows: FederatedReactionRow[];
    failed: boolean;
  }>({ channel: channelId, rows: [], failed: false });
  const request = useRef<AbortController | null>(null);
  const refresh = useCallback(() => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    void client
      .get<FederatedReactionRow[]>(
        `/api/channels/${encodeURIComponent(channelId)}/federated-reactions`,
        { signal: controller.signal },
      )
      .then((rows) => {
        if (!Array.isArray(rows)) throw new Error("Invalid federated reaction response");
        if (!controller.signal.aborted) setState({ channel: channelId, rows, failed: false });
      })
      .catch(() => {
        if (!controller.signal.aborted)
          setState((old) => ({
            channel: channelId,
            rows: old.channel === channelId ? old.rows : [],
            failed: true,
          }));
      });
  }, [client, channelId]);
  const schedule = useRef<() => void>(() => {});
  useEffect(() => {
    const scheduler = coalescedRefresh(refresh);
    schedule.current = scheduler.request;
    scheduler.request();
    return () => {
      schedule.current = () => {};
      scheduler.dispose();
      request.current?.abort();
    };
  }, [refresh]);
  const invalidate = useCallback(() => schedule.current(), []);
  useHomeLive(invalidate);
  return state.channel === channelId ? state : { channel: channelId, rows: [], failed: false };
}
