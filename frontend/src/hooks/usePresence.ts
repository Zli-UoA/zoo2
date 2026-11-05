import { useEffect, useMemo } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

/** Only a visible authenticated client renews the short presence lease. */
export function usePresence() {
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  useEffect(() => {
    if (!currentUserId) return;
    const controller = new AbortController();
    const heartbeat = () => {
      if (document.visibilityState === "visible")
        void client
          .request("/api/presence/heartbeat", {
            method: "POST",
            body: {},
            signal: controller.signal,
          })
          .catch(() => {});
    };
    heartbeat();
    const timer = setInterval(heartbeat, 30_000);
    document.addEventListener("visibilitychange", heartbeat);
    return () => {
      controller.abort();
      clearInterval(timer);
      document.removeEventListener("visibilitychange", heartbeat);
    };
  }, [client, currentUserId]);
}

export function useConversationView(channelId: string | null) {
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  useEffect(() => {
    if (!channelId || !currentUserId) return;
    void client
      .request(`/api/channels/${channelId}/view`, { method: "POST", body: { open: true } })
      .catch(() => {});
    // Closing the screen does not remove membership or channel access.
    return () => {
      void client
        .request(`/api/channels/${channelId}/view`, { method: "POST", body: { open: false } })
        .catch(() => {});
    };
  }, [channelId, client, currentUserId]);
}
