import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

export const MAX_BUBBLE_CHAT_LENGTH = 1000;
const THROTTLE_MS = 200;
const POLL_INTERVAL_MS = 1000;
const IDLE_TIMEOUT_MS = 15000;
const RETRY_INTERVAL_MS = 1000;
const REQUEST_TIMEOUT_MS = 5000;

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
  const [isSharing, setIsSharing] = useState(false);
  const updateRef = useRef<((value: string) => void) | null>(null);

  useEffect(() => {
    setEntries([]);
    setContentState("");
    setIsSharing(false);
    if (!enabled) return;

    const sessionId = crypto.randomUUID();
    const path = "/api/bubble";
    let cancelled = false;
    let seq = 0;
    let activeContent = "";
    let lastConfirmedContent = "";
    let hasSentActive = false;
    let lastSent = 0;
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    let idleTimer: ReturnType<typeof setTimeout> | undefined;
    let pollTimer: ReturnType<typeof setTimeout> | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let activeAbortController: AbortController | null = null;
    let lastSuccessfulPoll = Date.now();

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
          clearTimeout(retryTimer);
          retryTimer = undefined;

          const valueToSend = cancelled || activeContent.trim().length === 0 ? "" : activeContent;
          if (valueToSend.length > 0) {
            lastSent = Date.now();
          }
          const requestSeq = ++seq;
          const abortController = new AbortController();
          activeAbortController = abortController;
          const timeoutId = setTimeout(() => abortController.abort(), REQUEST_TIMEOUT_MS);

          try {
            await client.request(path, {
              method: "POST",
              body: { session_id: sessionId, seq: requestSeq, content: valueToSend },
              signal: abortController.signal,
            });
            lastConfirmedContent = valueToSend;
            if (valueToSend === "") {
              hasSentActive = false;
            }
          } catch {
            if (abortController.signal.aborted && cancelled) {
              return;
            }
            if (!cancelled && !hasPendingWrite && activeContent !== lastConfirmedContent) {
              clearTimeout(retryTimer);
              retryTimer = setTimeout(() => {
                retryTimer = undefined;
                hasPendingWrite = true;
                void flushWrites();
              }, RETRY_INTERVAL_MS);
            }
          } finally {
            clearTimeout(timeoutId);
            if (activeAbortController === abortController) {
              activeAbortController = null;
            }
          }
        }
      } finally {
        inFlight = false;
      }
    };

    const send = () => {
      clearTimeout(retryTimer);
      retryTimer = undefined;
      hasPendingWrite = true;
      void flushWrites();
    };

    const stop = () => {
      clearTimeout(flushTimer);
      clearTimeout(idleTimer);
      clearTimeout(retryTimer);
      retryTimer = undefined;
      activeContent = "";
      lastSent = 0;
      if (hasSentActive || lastConfirmedContent.length > 0) {
        send();
      }
    };

    const clearKeepalive = () => {
      clearTimeout(flushTimer);
      clearTimeout(idleTimer);
      clearTimeout(retryTimer);
      retryTimer = undefined;
      activeContent = "";
      if (activeAbortController) {
        activeAbortController.abort();
        activeAbortController = null;
      }
      if (hasSentActive || lastConfirmedContent.length > 0) {
        hasSentActive = false;
        lastConfirmedContent = "";
        const requestSeq = ++seq;
        void client
          .request(path, {
            method: "POST",
            body: { session_id: sessionId, seq: requestSeq, content: "" },
            keepalive: true,
          })
          .catch(() => {});
      }
    };

    updateRef.current = (raw: string) => {
      const next = clampBubbleChatContent(raw);
      setContentState(next);
      if (next.trim().length === 0 || document.visibilityState === "hidden") {
        setIsSharing(false);
        stop();
        return;
      }
      setIsSharing(true);
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
        setIsSharing(false);
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
            lastSuccessfulPoll = Date.now();
            setEntries(result);
          }
        } catch {
          // Keep previous entries on a transient network failure instead of wiping them out,
          // but if poll failures persist beyond IDLE_TIMEOUT_MS, server-side presence has expired.
          if (!cancelled && isVisible() && Date.now() - lastSuccessfulPoll >= IDLE_TIMEOUT_MS) {
            setEntries([]);
          }
        }
      }
      if (!cancelled) {
        pollTimer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
      }
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        setIsSharing(false);
        stop();
        setEntries([]);
      } else {
        clearTimeout(pollTimer);
        void poll();
      }
    };

    const onPageHide = () => {
      setIsSharing(false);
      clearKeepalive();
    };

    document.addEventListener("visibilitychange", onVisibilityChange);
    window.addEventListener("pagehide", onPageHide);
    void poll();

    return () => {
      cancelled = true;
      updateRef.current = null;
      setIsSharing(false);
      clearKeepalive();
      clearTimeout(pollTimer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.removeEventListener("pagehide", onPageHide);
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
    isSharing: enabled ? isSharing : false,
    setContent,
    clearContent,
  };
}
