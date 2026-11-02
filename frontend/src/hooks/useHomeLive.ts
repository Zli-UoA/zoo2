import { useEffect, useRef } from "react";
import { buildAuthHeaders } from "../lib/api/client";
import { useCollections } from "../lib/electric/collections-context";

/** Authenticated invalidations shared by Home and ActivityPub notification surfaces. */
export function useHomeLive(onRefresh: () => void) {
  const { config } = useCollections();
  const callback = useRef(onRefresh);
  useEffect(() => {
    callback.current = onRefresh;
  }, [onRefresh]);
  useEffect(() => {
    let stopped = false;
    let socket: WebSocket | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let pending: ReturnType<typeof setTimeout> | undefined;
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
          if (pending) return;
          pending = setTimeout(() => {
            pending = undefined;
            callback.current();
          }, 100);
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
        callback.current();
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
      clearTimeout(pending);
      socket?.close();
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("online", resume);
    };
  }, [config.apiBaseUrl, config.devUserId]);
}
