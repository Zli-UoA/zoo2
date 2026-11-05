import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiError, createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

export const MAX_BUBBLE_CHAT_LENGTH = 1000;
const THROTTLE_MS = 200;
const POLL_INTERVAL_MS = 1000;
const IDLE_TIMEOUT_MS = 15000;
const RETRY_INTERVAL_MS = 1000;
const REQUEST_TIMEOUT_MS = 5000;
const POLL_TIMEOUT_MS = 5000;

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
    let confirmedContent = "";
    let confirmedSeq = 0;
    let latestNonEmptySeq = 0;
    let serverMayHaveContent = false;
    let lastSent = 0;
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    let idleTimer: ReturnType<typeof setTimeout> | undefined;
    let pollTimer: ReturnType<typeof setTimeout> | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let activeAbortController: AbortController | null = null;
    let activePollAbortController: AbortController | null = null;
    let pollGeneration = 0;
    let lastSuccessfulPoll = Date.now();
    let serverTimeOffset = 0;

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
          const requestSeq = ++seq;
          if (valueToSend.length > 0) {
            latestNonEmptySeq = requestSeq;
            serverMayHaveContent = true;
            lastSent = Date.now();
          }

          const abortController = new AbortController();
          activeAbortController = abortController;
          const timeoutId = setTimeout(() => abortController.abort(), REQUEST_TIMEOUT_MS);

          try {
            const sentAt = Date.now() + serverTimeOffset;
            const res = await client.request<{
              ok?: boolean;
              stale?: boolean;
              server_time?: number;
            }>(path, {
              method: "POST",
              body: {
                session_id: sessionId,
                seq: requestSeq,
                content: valueToSend,
                sent_at: sentAt,
              },
              signal: abortController.signal,
            });
            if (typeof res?.server_time === "number") {
              serverTimeOffset = res.server_time - Date.now();
            }
            if (res?.ok !== true || res?.stale === true) {
              throw new Error("Write rejected as stale or unsuccessful");
            }
            confirmedSeq = requestSeq;
            confirmedContent = valueToSend;
            if (valueToSend === "" && requestSeq >= latestNonEmptySeq) {
              serverMayHaveContent = false;
            }
          } catch (err) {
            if (abortController.signal.aborted && cancelled) {
              return;
            }
            // Calibrate clock skew if server returns server_time in 400 Bad Request error response
            const errorBody =
              err instanceof ApiError ? err.body : (err as { body?: unknown } | undefined)?.body;
            if (
              typeof errorBody === "object" &&
              errorBody !== null &&
              "server_time" in errorBody &&
              typeof (errorBody as { server_time?: unknown }).server_time === "number"
            ) {
              serverTimeOffset = (errorBody as { server_time: number }).server_time - Date.now();
            }
            // If there is no immediate subsequent write pending and either the latest target content
            // is not confirmed or the server may still hold unconfirmed content, schedule a retry.
            const targetContent =
              cancelled || activeContent.trim().length === 0 ? "" : activeContent;
            const needsRetry =
              confirmedContent !== targetContent ||
              confirmedSeq < requestSeq ||
              (targetContent === "" && serverMayHaveContent);

            if (!cancelled && !hasPendingWrite && needsRetry) {
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
      if (serverMayHaveContent || confirmedContent.length > 0 || inFlight || hasPendingWrite) {
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

      if (serverMayHaveContent || confirmedContent.length > 0 || inFlight || hasPendingWrite) {
        serverMayHaveContent = false;
        confirmedContent = "";
        inFlight = false;
        hasPendingWrite = false;
        const requestSeq = ++seq;
        const sentAt = Date.now() + serverTimeOffset;
        const body = { session_id: sessionId, seq: requestSeq, content: "", sent_at: sentAt };
        void (
          client.keepalive
            ? client.keepalive(path, body)
            : client.request(path, { method: "POST", body, keepalive: true })
        ).catch(() => {});
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
      clearTimeout(pollTimer);
      pollTimer = undefined;
      if (!isVisible() || cancelled) {
        return;
      }

      const currentGen = ++pollGeneration;
      if (activePollAbortController) {
        activePollAbortController.abort();
      }
      const abortController = new AbortController();
      activePollAbortController = abortController;
      const pollTimeoutId = setTimeout(() => abortController.abort(), POLL_TIMEOUT_MS);

      try {
        const result = await client.get<BubbleChatEntry[]>(path, {
          signal: abortController.signal,
        });
        clearTimeout(pollTimeoutId);
        if (!cancelled && currentGen === pollGeneration && isVisible()) {
          lastSuccessfulPoll = Date.now();
          setEntries(result);
        }
      } catch {
        clearTimeout(pollTimeoutId);
        // Keep previous entries on a transient network failure instead of wiping them out,
        // but if poll failures persist beyond IDLE_TIMEOUT_MS, server-side presence has expired.
        if (!cancelled && currentGen === pollGeneration && isVisible()) {
          if (Date.now() - lastSuccessfulPoll >= IDLE_TIMEOUT_MS) {
            setEntries([]);
          }
        }
      } finally {
        clearTimeout(pollTimeoutId);
        if (activePollAbortController === abortController) {
          activePollAbortController = null;
        }
        if (!cancelled && currentGen === pollGeneration && isVisible()) {
          if (Date.now() - lastSuccessfulPoll >= IDLE_TIMEOUT_MS) {
            setEntries([]);
          }
          pollTimer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
        }
      }
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        clearTimeout(pollTimer);
        pollTimer = undefined;
        pollGeneration++;
        if (activePollAbortController) {
          activePollAbortController.abort();
          activePollAbortController = null;
        }
        setIsSharing(false);
        clearKeepalive();
        setEntries([]);
      } else {
        clearTimeout(pollTimer);
        pollTimer = undefined;
        pollGeneration++;
        if (activePollAbortController) {
          activePollAbortController.abort();
          activePollAbortController = null;
        }
        void poll();
      }
    };

    const onPageHide = () => {
      setIsSharing(false);
      clearKeepalive();
    };

    document.addEventListener("visibilitychange", onVisibilityChange);
    window.addEventListener("beforeunload", onPageHide, true);
    window.addEventListener("pagehide", onPageHide, true);
    void poll();

    return () => {
      cancelled = true;
      updateRef.current = null;
      setIsSharing(false);
      clearKeepalive();
      clearTimeout(pollTimer);
      pollGeneration++;
      if (activePollAbortController) {
        activePollAbortController.abort();
        activePollAbortController = null;
      }
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.removeEventListener("beforeunload", onPageHide, true);
      window.removeEventListener("pagehide", onPageHide, true);
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
