import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { getAuthSessionGeneration } from "../auth";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { createConfiguredApiClient } from "../api/client";
import { useCollections } from "../electric/collections-context";

const empty: string[] = [];
export interface EmojiHistory {
  read: () => string[];
  subscribe: (notify: () => void) => () => void;
  refresh: () => void;
  record: (emoji: string) => void;
}
const inactive: EmojiHistory = {
  read: () => empty,
  subscribe: () => () => {},
  refresh: () => {},
  record: () => {},
};
export const EmojiHistoryContext = createContext<EmojiHistory>(inactive);
export const useRecordEmoji = () => useContext(EmojiHistoryContext).record;

export function useRecentEmojis() {
  const history = useContext(EmojiHistoryContext);
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState !== "hidden") history.refresh();
    };
    refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => document.removeEventListener("visibilitychange", refresh);
  }, [history]);
  return useSyncExternalStore(history.subscribe, history.read, history.read);
}

export function EmojiHistoryProvider({ children }: { children: ReactNode }) {
  const { config } = useCollections();
  const { currentUserId } = useCurrentUser();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const history = useMemo(() => {
    let values = empty;
    let queue = Promise.resolve();
    let controller = new AbortController();
    const listeners = new Set<() => void>();
    const path = "/api/users/me/recent-emojis";
    const request = (emoji?: string) => {
      const { signal } = controller;
      const session = getAuthSessionGeneration();
      if (!currentUserId || signal.aborted) return;
      queue = queue
        .then(async () => {
          if (signal.aborted || session !== getAuthSessionGeneration()) return;
          const next =
            emoji === undefined
              ? await client.get<string[]>(path, { signal })
              : (await client.post<string[]>(path, { emoji }, { signal })).data;
          if (signal.aborted || session !== getAuthSessionGeneration()) return;
          values = next;
          listeners.forEach((notify) => notify());
        })
        .catch(() => {
          /* Refresh from the server when the picker opens again. */
        });
    };
    return {
      read: () => values,
      subscribe: (notify: () => void) => {
        listeners.add(notify);
        return () => {
          listeners.delete(notify);
        };
      },
      refresh: () => request(),
      record: (emoji: string) => request(emoji),
      start: () => {
        if (controller.signal.aborted) controller = new AbortController();
      },
      stop: () => controller.abort(),
    };
  }, [client, currentUserId]);
  useLayoutEffect(() => {
    history.start();
    return history.stop;
  }, [history]);
  return <EmojiHistoryContext.Provider value={history}>{children}</EmojiHistoryContext.Provider>;
}
