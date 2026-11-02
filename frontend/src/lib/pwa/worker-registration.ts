let registration: Promise<ServiceWorkerRegistration> | null = null;

/** Include the deployed CSP in both the SW registration URL and its cache identity. */
export function registerWorker(): Promise<ServiceWorkerRegistration> {
  if (!registration) {
    registration = (async () => {
      const existing = await navigator.serviceWorker.getRegistration("/");
      let policy: string;
      try {
        // Navigations are network-first in the SW, including when it already controls this page.
        const response = await fetch("/", { cache: "no-store" });
        if (!response.ok) throw new Error(`Could not read CSP: ${response.status}`);
        policy = response.headers.get("content-security-policy") ?? "";
      } catch (error) {
        // Offline startup must keep working with the installed shell and policy.
        if (existing) return existing;
        throw error;
      }
      const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(policy));
      const version = Array.from(new Uint8Array(digest), (byte) =>
        byte.toString(16).padStart(2, "0"),
      ).join("");
      const url = new URL(`/sw.js?v=${version}`, window.location.origin);
      if (existing?.active?.scriptURL === url.href) {
        await existing.update().catch(() => {});
        return existing;
      }
      return navigator.serviceWorker.register(url.href, { scope: "/", updateViaCache: "none" });
    })().catch((error: unknown) => {
      registration = null;
      throw error;
    });
  }
  return registration;
}

/** Finish the SW upgrade before SQLite fetches its Worker from the managed asset cache. */
export async function prepareOfflineWorker(): Promise<void> {
  if (!window.isSecureContext || !("serviceWorker" in navigator)) return;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopWaiting: (() => void) | undefined;
  let stopped = false;
  try {
    await Promise.race([
      (async () => {
        const registration = await registerWorker();
        if (stopped) return;
        const updating = registration.installing ?? registration.waiting;
        // A hard reload may bypass an already-active SW. Without an update,
        // activate/clients.claim will not run again, so control cannot change here.
        if (
          !updating &&
          registration.active?.state === "activated" &&
          navigator.serviceWorker.controller !== registration.active
        )
          return;
        const target = updating ?? registration.active;
        if (!target) return;
        await new Promise<void>((resolve, reject) => {
          const cleanup = () => {
            navigator.serviceWorker.removeEventListener("controllerchange", check);
            target.removeEventListener("statechange", check);
          };
          const check = () => {
            if (navigator.serviceWorker.controller === target) {
              cleanup();
              resolve();
            } else if (target.state === "redundant") {
              cleanup();
              reject(new Error("Service Worker update failed"));
            }
          };
          stopWaiting = cleanup;
          navigator.serviceWorker.addEventListener("controllerchange", check);
          target.addEventListener("statechange", check);
          check();
        });
      })(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("Service Worker update timed out")), 8_000);
      }),
    ]);
  } finally {
    stopped = true;
    if (timer !== undefined) clearTimeout(timer);
    stopWaiting?.();
  }
}
