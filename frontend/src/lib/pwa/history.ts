import { createBrowserHistory } from "@tanstack/react-router";

// Keep these markers in each entry so restarting the router or reloading an
// installed app preserves its history instead of adding another boundary.
declare module "@tanstack/react-router" {
  interface HistoryState {
    zooPwaHistory?: boolean;
    zooPwaBoundary?: boolean;
  }
}

export function createAppHistory() {
  const standalone =
    window.matchMedia("(display-mode: standalone)").matches ||
    (window.navigator as Navigator & { standalone?: boolean }).standalone === true;

  if (!standalone) return createBrowserHistory();

  const state = window.history.state ?? {};
  if (!state.zooPwaHistory || state.zooPwaBoundary) {
    // The app's first real entry remains index 0, so canGoBack() and existing
    // direct-link fallbacks don't mistake the boundary for an app screen.
    if (!state.zooPwaBoundary) {
      const key = crypto.randomUUID();
      window.history.replaceState(
        {
          ...state,
          key,
          __TSR_key: key,
          __TSR_index: -1,
          zooPwaHistory: true,
          zooPwaBoundary: true,
        },
        "",
      );
    }
    const key = crypto.randomUUID();
    window.history.pushState(
      { ...state, key, __TSR_key: key, __TSR_index: 0, zooPwaHistory: true, zooPwaBoundary: false },
      "",
    );
  }

  const history = createBrowserHistory();
  const push = history.push;
  const replace = history.replace;
  history.push = (path, state, options) =>
    push(path, { ...state, zooPwaHistory: true, zooPwaBoundary: false }, options);
  history.replace = (path, state, options) =>
    replace(path, { ...state, zooPwaHistory: true, zooPwaBoundary: false }, options);

  history.block({
    blockerFn: ({ nextLocation }) => nextLocation.state.zooPwaBoundary === true,
    // Login, external links and explicit reloads must remain possible. Only
    // same-document history traversal into the boundary is absorbed here.
    enableBeforeUnload: false,
  });
  return history;
}
