import { createRouter as createTanStackRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";

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
    scrollRestoration: true,
    defaultPreload: "intent",
    defaultPreloadStaleTime: 0,
  });

  return router;
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
