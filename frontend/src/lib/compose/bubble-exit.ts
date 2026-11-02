import { sendKeepaliveRequest } from "../api/client";

type ExitSnapshot = {
  session: string;
  offset: number;
  body: object;
  headers: Record<string, string>;
};

/** Start authenticated cleanup while the departing document can still initiate requests. */
export function createBubbleExit(url: string) {
  let snapshot: ExitSnapshot | undefined;
  let closed = false;
  const clear = () => {
    if (!snapshot) return;
    const current = snapshot;
    snapshot = undefined;
    void sendKeepaliveRequest(
      url,
      { ...current.body, sent_at: Date.now() + current.offset },
      current.headers,
    ).catch(() => {});
  };
  const hidden = () => {
    if (document.visibilityState === "hidden") clear();
  };
  // pagehide alone can run too late for a cross-origin keepalive to start during navigation.
  window.addEventListener("beforeunload", clear, true);
  window.addEventListener("pagehide", clear, true);
  document.addEventListener("visibilitychange", hidden, true);
  return {
    update(payload: string) {
      if (closed) return;
      const value = JSON.parse(payload);
      if (value.clear === "true") {
        if (snapshot?.session === value.session) snapshot = undefined;
      } else snapshot = value;
    },
    close() {
      if (closed) return;
      clear();
      closed = true;
      window.removeEventListener("beforeunload", clear, true);
      window.removeEventListener("pagehide", clear, true);
      document.removeEventListener("visibilitychange", hidden, true);
    },
  };
}
