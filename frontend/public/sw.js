/*
 * Offline app shell.
 *
 * Cache the static SPA shell and hashed build assets so a reload can start the
 * app without a network. Never cache API, auth, OIDC or event-stream responses:
 * chat data is persisted separately in OPFS by TanStack DB.
 */
const CACHE = "zoo-shell-v2";
const SHELL = [
  "/",
  "/index.html",
  "/offline.html",
  "/manifest.webmanifest",
  "/icons/icon-180.png",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
];
const CACHEABLE_PREFIXES = ["/assets/", "/icons/", "/emojibase-data/"];

function isPrivatePath(pathname) {
  return pathname.startsWith("/api/") || pathname.startsWith("/auth/");
}

function isCacheableAsset(url) {
  return (
    CACHEABLE_PREFIXES.some((prefix) => url.pathname.startsWith(prefix)) ||
    url.pathname === "/manifest.webmanifest"
  );
}

/**
 * Store a decoded response without transport encoding headers.
 *
 * `fetch()` hands us decoded bytes, but Cache API keeps the original
 * `content-encoding`. Serving that combination makes module scripts fail with
 * ERR_FAILED, so strip the encoding headers before caching.
 */
async function cacheResponse(cache, request, response) {
  const body = await response.arrayBuffer();
  const headers = new Headers(response.headers);
  for (const name of [
    "content-encoding",
    "content-length",
    "transfer-encoding",
    "cache-control",
    "etag",
    "last-modified",
    "expires",
    "pragma",
    "age",
    "date",
    "connection",
    "keep-alive",
    "vary",
    "set-cookie",
  ]) {
    headers.delete(name);
  }
  // The browser may otherwise revalidate a cached response with the network.
  headers.set("cache-control", "no-store");
  const sanitized = new Response(body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
  await cache.put(request, sanitized.clone());
  return sanitized;
}

/**
 * Cache the hashed JS/CSS referenced by the built index. The first page load
 * happens before this worker controls the page, so runtime caching alone would
 * leave an offline reload without its scripts.
 */
async function precacheIndexAssets(cache, response) {
  let html = "";
  try {
    html = await response.clone().text();
  } catch {
    return;
  }
  const assets = new Set();
  for (const match of html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)) {
    if (match[1]) assets.add(match[1]);
  }
  await Promise.all(
    [...assets].map(async (path) => {
      try {
        const asset = await fetch(path, { cache: "reload" });
        if (asset.ok) await cacheResponse(cache, path, asset);
      } catch {
        /* A later navigation can retry the asset. */
      }
    }),
  );
}

async function precacheShell() {
  const cache = await caches.open(CACHE);
  // Fetch each entry separately: one missing icon must not fail the install.
  await Promise.all(
    SHELL.map(async (path) => {
      try {
        const response = await fetch(path, { cache: "reload" });
        if (response.ok) {
          const cached = await cacheResponse(cache, path, response);
          if (path === "/index.html" || path === "/") {
            await precacheIndexAssets(cache, cached);
          }
        }
      } catch {
        /* Offline installs fall back to the runtime cache. */
      }
    }),
  );
}

async function handleNavigation(request) {
  try {
    const response = await fetch(request);
    if (response.ok) {
      // The SPA shell is static. Store it under /index.html so any navigation
      // can fall back to it, regardless of the deep link that was open.
      const cache = await caches.open(CACHE);
      return await cacheResponse(cache, "/index.html", response);
    }
    return response;
  } catch {
    const cache = await caches.open(CACHE);
    return (
      (await cache.match("/index.html")) ||
      (await cache.match("/")) ||
      (await cache.match("/offline.html")) ||
      Response.error()
    );
  }
}

async function handleAsset(request) {
  const cache = await caches.open(CACHE);
  const cached = await cache.match(request);
  if (cached) return cached;
  try {
    const response = await fetch(request);
    if (response.ok && response.type === "basic") {
      return await cacheResponse(cache, request, response);
    }
    return response;
  } catch {
    return Response.error();
  }
}

self.addEventListener("install", (event) => {
  event.waitUntil(precacheShell().then(() => self.skipWaiting()));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (name.startsWith("zoo-") && name !== CACHE) await caches.delete(name);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (event) => {
  const data = event.data;
  if (!data || data.type !== "CACHE_ASSETS" || !Array.isArray(data.urls)) return;
  const urls = data.urls
    .filter((url) => typeof url === "string" && url.startsWith("/assets/"))
    .slice(0, 200);
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      for (const url of urls) {
        if (await cache.match(url)) continue;
        try {
          const response = await fetch(url);
          if (response.ok && response.type === "basic") {
            await cacheResponse(cache, url, response);
          }
        } catch {
          /* A later load can retry the asset. */
        }
      }
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || event.request.method !== "GET") return;
  if (isPrivatePath(url.pathname) || url.pathname === "/sw.js") return;

  if (event.request.mode === "navigate") {
    event.respondWith(handleNavigation(event.request));
  } else if (isCacheableAsset(url)) {
    event.respondWith(handleAsset(event.request));
  }
});

self.addEventListener("push", (event) => {
  let payload = {};
  try {
    const data = event.data?.json();
    if (data && typeof data === "object") payload = data;
  } catch {
    // Keep displaying a generic alert for old or malformed deliveries.
  }
  const language = (self.navigator.language || "en").toLowerCase();
  const fallback = language.startsWith("ja")
    ? "新しい通知があります。Zooを開いて確認してください。"
    : language.startsWith("zh")
      ? "你有新通知。請開啟 Zoo 查看。"
      : "You have a new notification. Open Zoo to view it.";
  const title = typeof payload.title === "string" ? payload.title : "Zoo";
  let body = typeof payload.body === "string" ? payload.body : fallback;
  if (payload.notificationType === "reaction") {
    body =
      (language.startsWith("ja")
        ? "リアクション: "
        : language.startsWith("zh")
          ? "回應: "
          : "Reaction: ") + body;
  }
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      tag: typeof payload.tag === "string" ? payload.tag : "zoo-activity",
      data: { url: typeof payload.url === "string" ? payload.url : "/" },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      let target = "/";
      try {
        const url = new URL(event.notification.data?.url || "/", self.location.origin);
        if (url.origin === self.location.origin && url.pathname.startsWith("/channels/")) {
          target = url.href;
        }
      } catch {
        /* Invalid destinations open the app root. */
      }
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      for (const client of windows) {
        if (new URL(client.url).origin === self.location.origin) {
          try {
            if (target !== "/" && client.url !== target) await client.navigate(target);
            await client.focus();
            return;
          } catch {
            /* Try another app window or open a new one. */
          }
        }
      }
      await self.clients.openWindow(target);
    })(),
  );
});
