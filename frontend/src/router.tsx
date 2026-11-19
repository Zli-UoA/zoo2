import { createRouter as createTanStackRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";
import { createAppHistory } from "./lib/pwa/history";
import { NotFoundScreen } from "./components/layout/NotFoundScreen";

const RELOAD_GUARD_KEY = "zoo2:preload-error-reload-at";
const RELOAD_GUARD_MS = 60_000;

// After a deploy, hashed chunks of the old build no longer exist. Reload once
// to pick up the new index.html; the guard prevents a reload loop.
function installPreloadErrorReload() {
  window.addEventListener("vite:preloadError", (event) => {
    try {
      const last = Number(window.sessionStorage.getItem(RELOAD_GUARD_KEY) ?? 0);
      if (Date.now() - last < RELOAD_GUARD_MS) return;
      window.sessionStorage.setItem(RELOAD_GUARD_KEY, String(Date.now()));
    } catch {
      return;
    }
    event.preventDefault();
    window.location.reload();
  });
}

let preloadErrorReloadInstalled = false;

export function getRouter() {
  if (typeof window !== "undefined" && !preloadErrorReloadInstalled) {
    preloadErrorReloadInstalled = true;
    installPreloadErrorReload();
  }

  const router = createTanStackRouter({
    routeTree,
    history: typeof window !== "undefined" ? createAppHistory() : undefined,
    scrollRestoration: true,
    defaultPreload: "intent",
    defaultPreloadStaleTime: 0,
    // 未知のパスは _shell/$ の catch-all がシェル内で描画する。loader などで
    // notFound が投げられたときのフォールバックとしても同じ画面を使う。
    defaultNotFoundComponent: () => <NotFoundScreen />,
  });

  return router;
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
