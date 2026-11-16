import { useMemo, useSyncExternalStore } from "react";
import { createConfiguredApiClient, type ApiEnvelope } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";
import { THREADS_CHANGED } from "../lib/electric/threads";

export interface ThreadParticipant {
  id: string;
  user_name: string;
  display_name: string | null;
  avatar_url: string | null;
}
export interface ThreadMembership {
  id: string;
  channel_id: string;
  membership: "none" | "joined" | "left";
  thread_archive_at: string | null;
  can_participate: boolean;
  /** 現在参加中の人数と、先頭から最大50人。 */
  participant_count?: number;
  participants?: ThreadParticipant[];
}
type Snapshot = { data: ThreadMembership | null; pending: boolean; error: boolean };
const empty: Snapshot = { data: null, pending: false, error: false };
const stores = new WeakMap<object, Map<string, ReturnType<typeof createStore>>>();

// Only the viewed thread is subscribed. Consumers share the request, timer and
// mutation result; hidden tabs and unmounted/account-switched views stop fetching.
function createStore(
  config: Parameters<typeof createConfiguredApiClient>[0],
  id: string,
  userId: string,
) {
  const api = createConfiguredApiClient(config);
  const path = `/api/threads/${encodeURIComponent(id)}`;
  let state = empty;
  let generation = 0;
  let running = false;
  let queued = false;
  let controller: AbortController | undefined;
  const listeners = new Set<() => void>();
  let stop: (() => void) | undefined;
  const set = (next: Snapshot) => {
    state = next;
    listeners.forEach((listener) => listener());
  };
  const refresh = async () => {
    if (!listeners.size || document.visibilityState === "hidden") return;
    if (running || state.pending) {
      queued = true;
      return;
    }
    running = true;
    const started = generation;
    const request = new AbortController();
    controller = request;
    try {
      const result = await api.get<ApiEnvelope<ThreadMembership>>(path, {
        signal: AbortSignal.any([request.signal, AbortSignal.timeout(10_000)]),
      });
      if (!request.signal.aborted && started === generation)
        set({ data: result.data, pending: false, error: false });
    } catch {
      if (!request.signal.aborted && started === generation) set({ ...state, error: true });
    } finally {
      running = false;
      if (queued) {
        queued = false;
        void refresh();
      }
    }
  };
  return {
    snapshot: () => state,
    refresh,
    change: async (action: "join" | "leave" | "unarchive") => {
      if (state.pending) return;
      generation += 1;
      set({ ...state, pending: true, error: false });
      try {
        const result = await api.post<ThreadMembership>(
          `${path}/${action}`,
          {},
          { signal: AbortSignal.timeout(10_000) },
        );
        set({ data: result.data, pending: false, error: false });
        window.dispatchEvent(
          new CustomEvent(THREADS_CHANGED, { detail: { ...result.data, userId, config } }),
        );
      } catch {
        set({ ...state, pending: false, error: true });
      } finally {
        if (queued) {
          queued = false;
          void refresh();
        }
      }
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      if (listeners.size === 1) {
        void refresh();
        const changed = (event: Event) => {
          const detail = (event as CustomEvent).detail;
          if (detail?.userId && (detail.userId !== userId || detail.config !== config)) return;
          generation += 1;
          if (detail?.id === id) {
            set({ data: detail, pending: state.pending, error: false });
            return;
          }
          void refresh();
        };
        const visible = () => {
          if (document.visibilityState === "hidden") controller?.abort();
          else void refresh();
        };
        const timer = window.setInterval(() => void refresh(), 60_000);
        window.addEventListener(THREADS_CHANGED, changed);
        window.addEventListener("online", visible);
        window.addEventListener("focus", visible);
        document.addEventListener("visibilitychange", visible);
        stop = () => {
          generation += 1;
          controller?.abort();
          window.clearInterval(timer);
          window.removeEventListener(THREADS_CHANGED, changed);
          window.removeEventListener("online", visible);
          window.removeEventListener("focus", visible);
          document.removeEventListener("visibilitychange", visible);
        };
      }
      return () => {
        listeners.delete(listener);
        if (!listeners.size) stop?.();
      };
    },
  };
}
const noSubscribe = () => () => {};
export function useThreadMembership(userId: string | null, id: string | null) {
  const { config } = useCollections();
  const store = useMemo(() => {
    if (!id || !userId) return null;
    let scope = stores.get(config);
    if (!scope) {
      scope = new Map();
      stores.set(config, scope);
    }
    const key = JSON.stringify([userId, id]);
    let value = scope.get(key);
    if (!value) {
      value = createStore(config, id, userId);
      scope.set(key, value);
    }
    return value;
  }, [config, userId, id]);
  const state = useSyncExternalStore(
    store?.subscribe ?? noSubscribe,
    store?.snapshot ?? (() => empty),
    () => empty,
  );
  return { ...state, change: store?.change, retry: store?.refresh };
}
