import { visiblePoll } from "../lib/visiblePoll";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

export interface TypingUser {
  id: string;
  display_name: string;
}

/** Ephemeral presence, separate from replicated messages. */
export function useTyping(channelId: string, parentId: string | null, enabled: boolean) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [users, setUsers] = useState<TypingUser[]>([]);
  const activity = useRef<((active: boolean) => void) | null>(null);
  useEffect(() => {
    setUsers([]);
    if (!enabled) return;
    const sessionId = crypto.randomUUID();
    const path = `/api/channels/${encodeURIComponent(channelId)}/typing`;
    const query = parentId ? `?parent_id=${encodeURIComponent(parentId)}` : "";
    let cancelled = false;
    let active = false;
    let lastSent = 0;
    let stopTimer: ReturnType<typeof setTimeout> | undefined;

    // Serialize writes so a delayed start cannot overtake a stop.
    let writes = Promise.resolve();
    const send = (value: boolean) => {
      writes = writes
        .then(async () => {
          if (value && (cancelled || !active)) return;
          await client.request(path, {
            method: "POST",
            body: { parent_id: parentId, session_id: sessionId, active: value },
            signal: AbortSignal.timeout(5000),
          });
        })
        .catch(() => {});
    };
    const stop = () => {
      clearTimeout(stopTimer);
      if (active) send(false);
      active = false;
      lastSent = 0;
    };
    activity.current = (value) => {
      if (!value || document.visibilityState === "hidden") {
        stop();
        return;
      }
      const started = !active;
      active = true;
      if (started) polling.refresh();
      const now = Date.now();
      if (!lastSent || now - lastSent >= 2000) {
        send(true);
        lastSent = now;
      }
      clearTimeout(stopTimer);
      stopTimer = setTimeout(stop, 3000);
    };
    const isVisible = () => document.visibilityState !== "hidden";
    const polling = visiblePoll(
      async (signal) => {
        try {
          const result = await client.get<TypingUser[]>(path + query, { signal });
          if (!cancelled && !signal.aborted && isVisible()) setUsers(result);
          return result.length > 0 || active;
        } catch {
          if (!cancelled && isVisible()) setUsers([]);
        }
      },
      { interval: 1500, maxInterval: 5000 },
    );
    const onHidden = () => {
      if (document.visibilityState === "hidden") {
        stop();
        setUsers([]);
      }
    };
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", stop);
    return () => {
      cancelled = true;
      activity.current = null;
      stop();
      polling.dispose();
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", stop);
    };
  }, [client, channelId, parentId, enabled]);
  const onTyping = useCallback((active: boolean) => activity.current?.(active), []);
  return { users: enabled ? users : [], onTyping };
}
