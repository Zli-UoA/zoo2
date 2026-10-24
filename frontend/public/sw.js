/* Cache only public offline assets. Never persist chat, API, OIDC or authenticated HTML. */
const CACHE = "zoo-offline-v1";
const ASSETS = ["/offline.html", "/icons/icon-192.png", "/icons/icon-512.png"];
self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE).then((cache) => cache.addAll(ASSETS)));
});
self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (name.startsWith("zoo-offline-") && name !== CACHE) await caches.delete(name);
      }
      await self.clients.claim();
    })(),
  );
});
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || event.request.method !== "GET") return;
  if (
    event.request.mode === "navigate" &&
    !url.pathname.startsWith("/api/") &&
    !url.pathname.startsWith("/auth/")
  ) {
    event.respondWith(
      fetch(event.request).catch(
        async () => (await caches.match("/offline.html")) || Response.error(),
      ),
    );
  } else if (ASSETS.includes(url.pathname)) {
    event.respondWith(caches.match(event.request).then((cached) => cached || fetch(event.request)));
  }
});
self.addEventListener("push", (event) => {
  // Empty-body Web Push: message content and credentials are never sent to the push provider.
  const language = (self.navigator.language || "en").toLowerCase();
  const body = language.startsWith("ja")
    ? "新しい通知があります。Zooを開いて確認してください。"
    : language.startsWith("zh")
      ? "你有新通知。請開啟 Zoo 查看。"
      : "You have a new notification. Open Zoo to view it.";
  event.waitUntil(
    self.registration.showNotification("Zoo", {
      body,
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      tag: "zoo-activity",
    }),
  );
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      for (const client of windows) {
        if (new URL(client.url).origin === self.location.origin) {
          await client.focus();
          return;
        }
      }
      await self.clients.openWindow("/");
    })(),
  );
});
