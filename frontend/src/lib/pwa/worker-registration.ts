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
        void existing.update().catch(() => {});
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
