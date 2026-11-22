import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { createConfiguredApiClient } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { useCurrentUser } from "./useCurrentUser";

export const READ_STATE_CHANGED = "zoo-read-state-changed";

export interface ChannelReadState {
  channel_id: string;
  last_read_at: string | null;
  has_unread: boolean;
  /** Capped at 1000; `unread_count_truncated` tells the UI to show "1000+". */
  unread_count: number;
  unread_count_truncated: boolean;
  /** Oldest unread root message from somebody else, or null when fully read. */
  first_unread_at: string | null;
}

/** Older first; channels without a known boundary go last. */
function compareByFirstUnread(a: ChannelReadState, b: ChannelReadState): number {
  if (a.first_unread_at === b.first_unread_at) return a.channel_id.localeCompare(b.channel_id);
  if (a.first_unread_at === null) return 1;
  if (b.first_unread_at === null) return -1;
  return a.first_unread_at < b.first_unread_at ? -1 : 1;
}

function useUnreadFeed() {
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [data, setData] = useState<{ userId: string; states: ChannelReadState[] } | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isError, setIsError] = useState(false);
  const refreshRef = useRef<() => void>(() => {});

  useEffect(() => {
    if (!currentUserId) {
      setData(null);
      setIsError(false);
      return;
    }
    let active = true;
    let inFlight = false;
    let pending = false;
    const load = async () => {
      if (document.visibilityState === "hidden") {
        pending = true;
        return;
      }
      if (inFlight) {
        pending = true;
        return;
      }
      inFlight = true;
      pending = false;
      setIsLoading(true);
      try {
        const states = await client.get<ChannelReadState[]>("/api/channel-read-states");
        if (!active) return;
        setData({ userId: currentUserId, states });
        setIsError(false);
      } catch {
        if (active) setIsError(true);
      } finally {
        inFlight = false;
        if (active) setIsLoading(false);
        // 取得中に届いた更新は1回だけまとめて反映する。
        if (active && pending) void load();
      }
    };
    refreshRef.current = () => void load();
    void load();
    const interval = window.setInterval(() => void load(), 10000);
    const onFocus = () => void load();
    const onVisibility = () => {
      if (document.visibilityState !== "hidden") void load();
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener(READ_STATE_CHANGED, onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      active = false;
      refreshRef.current = () => {};
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener(READ_STATE_CHANGED, onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [client, currentUserId]);

  const refresh = useCallback(() => refreshRef.current(), []);
  const applyRead = useCallback((channelId: string) => {
    setData((current) =>
      current
        ? {
            ...current,
            states: current.states.map((state) =>
              state.channel_id === channelId
                ? {
                    ...state,
                    has_unread: false,
                    unread_count: 0,
                    unread_count_truncated: false,
                    first_unread_at: null,
                  }
                : state,
            ),
          }
        : current,
    );
  }, []);

  const states = data?.userId === currentUserId ? data.states : [];
  const unreadStates = useMemo(
    () => states.filter((state) => state.has_unread).sort(compareByFirstUnread),
    [states],
  );
  const unreadChannelIds = useMemo(
    () => new Set(unreadStates.map((state) => state.channel_id)),
    [unreadStates],
  );

  return {
    states,
    unreadStates,
    unreadChannelIds,
    unreadConversationCount: unreadStates.length,
    isReady: data?.userId === currentUserId,
    isLoading,
    isError,
    refresh,
    applyRead,
  };
}

const UnreadContext = createContext<ReturnType<typeof useUnreadFeed> | null>(null);

function UnreadFeedProvider({ children }: { children: ReactNode }) {
  const value = useUnreadFeed();
  return createElement(UnreadContext.Provider, { value }, children);
}

/** One unread fetch and polling loop for the rail, sidebar and catch-up screen. */
export function UnreadProvider({ children }: { children: ReactNode }) {
  const { currentUserId } = useCurrentUser();
  const { config } = useCollections();
  return createElement(UnreadFeedProvider, {
    key: JSON.stringify([config.apiBaseUrl, config.devUserId, currentUserId]),
    children,
  });
}

export function useUnread() {
  const unread = useContext(UnreadContext);
  if (!unread) throw new Error("UnreadProvider is required");
  return unread;
}
