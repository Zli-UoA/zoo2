import { createBrowserHistory } from "@tanstack/react-router";

export const PWA_HISTORY_SESSION_KEY = "zoo2:pwa-history";

interface PwaSession {
  boundaryKey: string;
  lastIndex: number;
}

export function createAppHistory() {
  const standalone =
    window.matchMedia("(display-mode: standalone)").matches ||
    (window.navigator as Navigator & { standalone?: boolean }).standalone === true;
  if (!standalone) return createBrowserHistory();

  // Session storage survives same-window document navigations (push
  // notifications and OIDC callbacks). Do not insert a new boundary there.
  let session: PwaSession | undefined;
  try {
    const saved = JSON.parse(window.sessionStorage.getItem(PWA_HISTORY_SESSION_KEY) ?? "null");
    if (typeof saved?.boundaryKey === "string" && Number.isInteger(saved.lastIndex)) {
      session = saved;
    }
    if (!session) {
      session = { boundaryKey: crypto.randomUUID(), lastIndex: 0 };
      window.sessionStorage.setItem(PWA_HISTORY_SESSION_KEY, JSON.stringify(session));
      window.history.replaceState(
        {
          ...window.history.state,
          key: session.boundaryKey,
          __TSR_key: session.boundaryKey,
          __TSR_index: -1,
        },
        "",
      );
      const key = crypto.randomUUID();
      window.history.pushState(
        { ...window.history.state, key, __TSR_key: key, __TSR_index: 0 },
        "",
      );
    } else if (window.history.state?.__TSR_key === session.boundaryKey) {
      // A browser may restore the boundary as a new document after eviction.
      window.history.forward();
    } else if (!window.history.state?.__TSR_key) {
      // A new document has no router state. Keep the app's logical index
      // increasing so its Back buttons can reach the previous app document.
      const key = crypto.randomUUID();
      window.history.replaceState(
        { ...window.history.state, key, __TSR_key: key, __TSR_index: session.lastIndex + 1 },
        "",
      );
    }
  } catch {
    // Without session storage we cannot identify a boundary across documents.
    return createBrowserHistory();
  }

  const { boundaryKey } = session;
  let restoringTo: string | undefined;
  const onPopState = (event: PopStateEvent) => {
    const key = event.state?.__TSR_key;
    if (key === boundaryKey) {
      // Catch only the boundary before TanStack sees it. A router blocker
      // would also make ordinary push/replace asynchronous and race their indexes.
      event.stopImmediatePropagation();
      restoringTo = history.location.state.__TSR_key;
      window.history.forward();
    } else if (restoringTo) {
      const restored = key === restoringTo;
      restoringTo = undefined;
      if (restored) event.stopImmediatePropagation();
    }
  };
  window.addEventListener("popstate", onPopState, { capture: true });
  const history = createBrowserHistory();
  const rememberIndex = () => {
    try {
      window.sessionStorage.setItem(
        PWA_HISTORY_SESSION_KEY,
        JSON.stringify({ boundaryKey, lastIndex: history.location.state.__TSR_index }),
      );
    } catch {
      // Storage failures must not prevent navigation in an already running app.
    }
  };
  rememberIndex();
  const unsubscribe = history.subscribe(rememberIndex);
  const destroy = history.destroy;
  history.destroy = () => {
    unsubscribe();
    window.removeEventListener("popstate", onPopState, { capture: true });
    destroy();
  };
  return history;
}
