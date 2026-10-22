import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";

import { READ_STATE_CHANGED, type ChannelReadState } from "../../../hooks/useUnreadChannels";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { getCollections } from "../../../lib/electric/collections";
import { parseUtcTimestamp } from "../../../lib/timestamps";
import type { FeedMessage } from "./messageFormat";

export function firstUnreadMessage(
  messages: FeedMessage[],
  userId: string,
  lastRead: number,
): string | null {
  return (
    messages.find(
      (message) =>
        message.user_id !== userId && parseUtcTimestamp(message.created_at).getTime() > lastRead,
    )?.id ?? null
  );
}

/** Keep the visit's boundary visible while advancing the server's read position. */
export function useMessageReadState(
  channelId: string | undefined,
  userId: string | null,
  messages: FeedMessage[],
  pending: boolean,
  writePending = false,
) {
  const { t } = useTranslation();
  const enabled = !!channelId && !!userId;
  const client = useMemo(
    () => (enabled ? createConfiguredApiClient(getCollections().config) : null),
    [enabled, userId],
  );
  const path = `/api/channels/${channelId}/read-state`;
  const [lastRead, setLastRead] = useState<number | null>(null);
  const [isError, setIsError] = useState(false);
  const saved = useRef(0);
  const inFlight = useRef(false);
  const [savedVersion, setSavedVersion] = useState(0);
  useEffect(() => {
    if (!client) return;
    let active = true;
    let loaded = false;
    const load = async () => {
      try {
        const state = await client.get<ChannelReadState>(path);
        if (!active) return;
        const timestamp = state.last_read_at ? parseUtcTimestamp(state.last_read_at).getTime() : 0;
        loaded = true;
        saved.current = timestamp;
        setLastRead(timestamp);
        setIsError(false);
      } catch {
        if (active) setIsError(true);
      }
    };
    void load();
    // A failed initial load can be retried by returning to the tab.
    const retry = () => {
      if (active && !loaded) void load();
    };
    window.addEventListener("focus", retry);
    return () => {
      active = false;
      window.removeEventListener("focus", retry);
    };
  }, [client, path]);
  const firstUnreadId =
    enabled && userId && !pending && lastRead !== null
      ? firstUnreadMessage(messages, userId, lastRead)
      : null;
  const markRead = useCallback(() => {
    if (
      !client ||
      pending ||
      writePending ||
      lastRead === null ||
      inFlight.current ||
      document.visibilityState === "hidden" ||
      !document.hasFocus()
    )
      return;
    const latest = messages.at(-1);
    if (!latest) return;
    const timestamp = parseUtcTimestamp(latest.created_at).getTime();
    if (!Number.isFinite(timestamp) || timestamp <= saved.current) return;
    inFlight.current = true;
    void client
      .post<ChannelReadState>(path, { message_id: latest.id })
      .then(({ data }) => {
        saved.current = data.last_read_at
          ? parseUtcTimestamp(data.last_read_at).getTime()
          : timestamp;
        window.dispatchEvent(new Event(READ_STATE_CHANGED));
        setSavedVersion((version) => version + 1);
      })
      .catch(() => {
        toast.error(t("common.writeError"), { id: "read-state-write-error" });
      })
      .finally(() => {
        inFlight.current = false;
      });
  }, [client, path, messages, pending, writePending, lastRead, t, savedVersion]);
  return { firstUnreadId, markRead, isLoading: enabled && lastRead === null && !isError, isError };
}
