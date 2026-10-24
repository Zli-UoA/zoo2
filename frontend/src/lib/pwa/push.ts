import { createConfiguredApiClient } from "../api/client";
import type { AppConfig } from "../config";

let registration: Promise<ServiceWorkerRegistration> | null = null;

export function supportsPush(): boolean {
  return (
    window.isSecureContext &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

export function registerWorker(): Promise<ServiceWorkerRegistration> {
  if (!registration) {
    registration = navigator.serviceWorker
      .register("/sw.js", { scope: "/", updateViaCache: "none" })
      .catch((error: unknown) => {
        registration = null;
        throw error;
      });
  }
  return registration;
}

export function applicationServerKey(value: string): Uint8Array<ArrayBuffer> {
  const decoded = atob(
    value.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (value.length % 4)) % 4),
  );
  return Uint8Array.from(decoded, (char) => char.charCodeAt(0));
}

export async function saveSubscription(
  config: AppConfig,
  subscription: PushSubscription,
): Promise<void> {
  const keys = subscription.toJSON().keys;
  if (!keys?.p256dh || !keys.auth) throw new Error("Missing push subscription keys");
  await createConfiguredApiClient(config).request("/api/push/subscriptions", {
    method: "POST",
    body: { endpoint: subscription.endpoint, keys },
  });
}

export async function disablePush(config: AppConfig): Promise<void> {
  if (!("serviceWorker" in navigator)) return;
  const worker = await navigator.serviceWorker.getRegistration("/");
  const subscription = await worker?.pushManager.getSubscription();
  if (!subscription) return;
  // Revoke locally even when the API is unavailable, before credentials are cleared at logout.
  const unsubscribed = await subscription.unsubscribe();
  if (!unsubscribed) throw new Error("Could not revoke push subscription");
  await createConfiguredApiClient(config).request("/api/push/subscriptions", {
    method: "DELETE",
    body: { endpoint: subscription.endpoint },
  });
}
