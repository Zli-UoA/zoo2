import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { coalescedRefresh } from "../lib/coalescedRefresh";
import { buildAuthHeaders } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

function useHomeLiveConnection(onRefresh: () => void) {
  const { config } = useCollections();
  const callback = useRef(onRefresh);
  useEffect(() => {
    callback.current = onRefresh;
  }, [onRefresh]);
  useEffect(() => {
    let stopped = false;
    let socket: WebSocket | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const pending = coalescedRefresh(() => {
      if (document.visibilityState !== "visible") {
        return;
      }
      callback.current();
    }, 1000);
    let attempts = 0;
    let connecting = false;
    const connect = async () => {
      if (stopped || connecting || (socket && socket.readyState !== WebSocket.CLOSED)) return;
      connecting = true;
      try {
        const headers = await buildAuthHeaders({ devUserId: config.devUserId });
        if (stopped) return;
        const url = new URL(`${config.apiBaseUrl.replace(/\/$/, "")}/api/home/live`, location.href);
        url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
        const connection = new WebSocket(url);
        socket = connection;
        connection.onopen = () =>
          connection.send(JSON.stringify(Object.fromEntries(headers.entries())));
        connection.onmessage = (event) => {
          if (event.data !== "refresh") return;
          attempts = 0;
          if (document.visibilityState === "visible") pending.request();
        };
        connection.onclose = () => {
          if (!stopped)
            retry = setTimeout(() => void connect(), Math.min(1000 * 2 ** attempts++, 15000));
        };
      } catch {
        if (!stopped) retry = setTimeout(() => void connect(), 3000);
      } finally {
        connecting = false;
      }
    };
    void connect();
    const resume = () => {
      if (document.visibilityState === "visible") {
        pending.request();
        if (!socket || socket.readyState === WebSocket.CLOSED) {
          clearTimeout(retry);
          void connect();
        }
      }
    };
    document.addEventListener("visibilitychange", resume);
    window.addEventListener("online", resume);
    return () => {
      stopped = true;
      clearTimeout(retry);
      pending.dispose();
      socket?.close();
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("online", resume);
    };
  }, [config.apiBaseUrl, config.devUserId]);
}

const HomeLiveContext = createContext<Set<() => void> | null>(null);

/** Share one authenticated connection across all Home and federation consumers. */
export function HomeLiveProvider({ children }: { children: ReactNode }) {
  const [listeners] = useState(() => new Set<() => void>());
  const refresh = useCallback(() => {
    for (const listener of listeners) listener();
  }, [listeners]);
  useHomeLiveConnection(refresh);
  return createElement(HomeLiveContext.Provider, { value: listeners }, children);
}

export function useHomeLive(onRefresh: () => void) {
  const listeners = useContext(HomeLiveContext);
  if (!listeners) throw new Error("HomeLiveProvider is required");
  const callback = useRef(onRefresh);
  useEffect(() => {
    callback.current = onRefresh;
  }, [onRefresh]);
  useEffect(() => {
    const listener = () => callback.current();
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, [listeners]);
}
