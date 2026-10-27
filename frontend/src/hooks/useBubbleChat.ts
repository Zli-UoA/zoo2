import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

export const MAX_BUBBLE_CHAT_LENGTH = 1000;
const THROTTLE_MS = 200;
const POLL_INTERVAL_MS = 1000;
const IDLE_TIMEOUT_MS = 15000;

export interface BubbleChatEntry {
  user_id: string;
  user_name: string;
  display_name: string;
  content: string;
}

export function clampBubbleChatContent(value: string): string {
  const chars = Array.from(value);
  return chars.length > MAX_BUBBLE_CHAT_LENGTH
    ? chars.slice(0, MAX_BUBBLE_CHAT_LENGTH).join("")
    : value;
}

/**
 * Ephemeral real-time bubble chat room state.
 * Active typing drafts are shared without a send step and disappear when cleared, idle, or leaving.
 */
export function useBubbleChat(enabled = true) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [entries, setEntries] = useState<BubbleChatEntry[]>([]);
  const [content, setContentState] = useState("");
  const updateRef = useRef<((value: string) => void) | null>(null);

  useEffect(() => {
    setEntries([]);
    setContentState("");
    if (!enabled) return;

    const sessionId = crypto.randomUUID();
    const path = "/api/bubble-chat";
    let cancelled = false;
    let activeContent = "";
    let hasSentActive = false;
    let lastSent = 0;
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    let idleTimer: ReturnType<typeof setTimeout> | undefined;
    let pollTimer: ReturnType<typeof setTimeout> | undefined;

    // Coalesce pending writes so at most one follow-up request is queued while a write is in flight,
    // always sending the latest activeContent (or empty string on clear) when the in-flight write finishes.
    let inFlight = false;
    let hasPendingWrite = false;

    const flushWrites = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        while (hasPendingWrite) {
          hasPendingWrite = false;
          clearTimeout(flushTimer);
          const valueToSend = cancelled || activeContent.trim().length === 0 ? "" : activeContent;
          if (valueToSend.length > 0) {
            lastSent = Date.now();
          }
          try {
            await client.request(path, {
              method: "POST",
              body: { session_id: sessionId, content: valueToSend },
              signal: AbortSignal.timeout(5000),
            });
          } catch {
            // Ignore transient network errors; subsequent input or timeout will recover.
          }
        }
      } finally {
        inFlight = false;
      }
    };

    const send = () => {
      hasPendingWrite = true;
      void flushWrites();
    };

    const stop = () => {
      clearTimeout(flushTimer);
      clearTimeout(idleTimer);
      activeContent = "";
      lastSent = 0;
      if (hasSentActive) {
        hasSentActive = false;
        send();
      }
    };

    updateRef.current = (raw: string) => {
      const next = clampBubbleChatContent(raw);
      setContentState(next);
      if (next.trim().length === 0 || document.visibilityState === "hidden") {
        stop();
        return;
      }
      activeContent = next;
      hasSentActive = true;
      const now = Date.now();
      clearTimeout(flushTimer);
      if (!lastSent || now - lastSent >= THROTTLE_MS) {
        lastSent = now;
        send();
      } else {
        flushTimer = setTimeout(
          () => {
            lastSent = Date.now();
            send();
          },
          THROTTLE_MS - (now - lastSent),
        );
      }
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        if (!cancelled) {
          setContentState("");
        }
        stop();
      }, IDLE_TIMEOUT_MS);
    };

    const isVisible = () => document.visibilityState !== "hidden";
    const poll = async () => {
      if (isVisible()) {
        try {
          const result = await client.get<BubbleChatEntry[]>(path, {
            signal: AbortSignal.timeout(5000),
          });
          if (!cancelled && isVisible()) {
            setEntries(result);
          }
        } catch {
          if (!cancelled) {
            setEntries([]);
          }
        }
      }
      if (!cancelled) {
        pollTimer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
      }
    };

    const onHidden = () => {
      if (document.visibilityState === "hidden") {
        setContentState("");
        stop();
        setEntries([]);
      }
    };

    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", stop);
    void poll();

    return () => {
      cancelled = true;
      updateRef.current = null;
      stop();
      clearTimeout(pollTimer);
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", stop);
    };
  }, [client, enabled]);

  const setContent = useCallback((value: string) => {
    updateRef.current?.(value);
  }, []);

  const clearContent = useCallback(() => {
    updateRef.current?.("");
  }, []);

  return {
    entries: enabled ? entries : [],
    content: enabled ? content : "",
    setContent,
    clearContent,
  };
}
