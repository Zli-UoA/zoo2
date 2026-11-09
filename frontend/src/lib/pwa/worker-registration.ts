let registration: Promise<ServiceWorkerRegistration> | null = null;

/** A cached document must not open its old SQLite Worker after an update purges its assets. */
export async function reloadIfShellUpdated(): Promise<boolean> {
  const entry = document.querySelector<HTMLScriptElement>(
    'script[type="module"][src*="/assets/index-"]',
  );
  const controller = navigator.serviceWorker?.controller;
  if (!entry || !controller) return false;
  const latest = await new Promise<string | null>((resolve) => {
    const channel = new MessageChannel();
    const finish = (value: string | null) => {
      clearTimeout(timer);
      channel.port1.close();
      channel.port2.close();
      resolve(value);
    };
    const timer = setTimeout(() => finish(null), 1_000);
    channel.port1.onmessage = (event) =>
      finish(typeof event.data?.entry === "string" ? event.data.entry : null);
    try {
      controller.postMessage({ type: "SHELL_ENTRY" }, [channel.port2]);
    } catch {
      finish(null);
    }
  });
  if (!latest) return false;
  const reloadKey = "zoo.pwa.shell-reload";
  const target = new URL(latest, window.location.origin).href;
  if (target === entry.src) {
    try {
      sessionStorage.removeItem(reloadKey);
    } catch {
      /* Storage can be disabled. */
    }
    return false;
  }
  // A fresh network document may be newer than a cache that failed to update.
  // Allow one reload per mismatch; never loop if storage/quota failures persist.
  const mismatch = JSON.stringify([entry.src, target]);
  try {
    if (sessionStorage.getItem(reloadKey) === mismatch) return false;
    sessionStorage.setItem(reloadKey, mismatch);
  } catch {
    return false;
  }
  window.location.reload();
  return true;
}

/** Include the deployed CSP in both the SW registration URL and its cache identity. */
export function registerWorker(): Promise<ServiceWorkerRegistration> {
  if (!registration) {
    registration = (async () => {
      const existing = await navigator.serviceWorker.getRegistration("/");
      let policy: string;
      try {
        // Read the deployed policy directly; navigation may use the installed shell.
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
