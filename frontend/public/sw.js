/*
 * Offline app shell.
 *
 * Cache the static SPA shell and hashed build assets so a reload can start the
 * app without a network. Never cache API, auth, OIDC or event-stream responses:
 * chat data is persisted separately in OPFS by TanStack DB.
 */
const POLICY_VERSION = new URL(
  self.location.href || "/sw.js",
  self.location.origin,
).searchParams.get("v");
const POLICY_PREFIX = POLICY_VERSION ? `zoo-shell-${POLICY_VERSION}-` : "zoo-shell-";
const DEFAULT_CACHE = `${POLICY_PREFIX}v2`;
const CACHE_PREFIX = "zoo-shell-";
const META_PATH = "/__zoo_shell_meta__";
// ビルドごとに変わる hashed entry を cache 名に含め、古い assets ごと
// activate で破棄できるようにする。名前は global に保持せず、毎回
// Cache API のメタ情報から決める（Service Worker は頻繁に停止されるため）。
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

function cacheNameForIndex(html) {
  const match = html.match(/\/assets\/index-([^"'?#]+)\.js/);
  return match?.[1] ? `${POLICY_PREFIX}${match[1]}` : DEFAULT_CACHE;
}

/**
 * 現在有効な app shell キャッシュを返す。install が書いたメタ情報のうち
 * 最新のものを選ぶ。メタ情報がない旧キャッシュは index.html の有無で拾う。
 */
async function findActiveShellCache() {
  const names = (await caches.keys()).filter((name) => name.startsWith(POLICY_PREFIX));
  let latest = null;
  for (const name of names) {
    const cache = await caches.open(name);
    const meta = await cache.match(META_PATH);
    if (!meta) continue;
    const installedAt = Number(await meta.text());
    if (Number.isFinite(installedAt) && (!latest || installedAt > latest.installedAt)) {
      latest = { name, installedAt };
    }
  }
  if (latest) return latest.name;
  for (const name of names) {
    const cache = await caches.open(name);
    if (await cache.match("/index.html")) return name;
  }
  return DEFAULT_CACHE;
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

async function fetchShellIndex() {
  for (const path of ["/index.html", "/"]) {
    try {
      const response = await fetch(path, { cache: "reload" });
      if (response.ok) return response;
    } catch {
      /* Try the other spelling of the entry point. */
    }
  }
  return null;
}

async function precacheShell() {
  const index = await fetchShellIndex();
  // Without the app shell there is nothing safe to replace; keep the old cache.
  if (!index) throw new Error("Cannot install an app shell without a fresh index");
  let html = "";
  try {
    html = await index.clone().text();
  } catch {
    /* Keep the default cache name when the body cannot be read. */
  }
  const name = cacheNameForIndex(html);
  const cache = await caches.open(name);
  const cached = await cacheResponse(cache, "/index.html", index);
  // Navigation fallbacks check /index.html first, but keep / for old clients.
  await cache.put("/", cached.clone());
  await precacheIndexAssets(cache, cached.clone());
  // Fetch each entry separately: one missing icon must not fail the install.
  await Promise.all(
    SHELL.filter((path) => path !== "/" && path !== "/index.html").map(async (path) => {
      try {
        const response = await fetch(path, { cache: "reload" });
        if (response.ok) await cacheResponse(cache, path, response);
      } catch {
        /* Offline installs fall back to the runtime cache. */
      }
    }),
  );
  // Write the marker last so a partially precached cache is not selected.
  await cache.put(META_PATH, new Response(String(Date.now())));
}

async function handleNavigation(request) {
  try {
    const response = await fetch(request);
    const contentType = response.headers.get("content-type") || "";
    if (response.ok && contentType.includes("text/html")) {
      // The SPA shell is static. Store it under /index.html so any navigation
      // can fall back to it, regardless of the deep link that was open.
      // Direct navigations to icons or JSON must not overwrite the shell.
      const cache = await caches.open(await findActiveShellCache());
      return await cacheResponse(cache, "/index.html", response);
    }
    return response;
  } catch {
    const cache = await caches.open(await findActiveShellCache());
    return (
      (await cache.match("/index.html")) ||
      (await cache.match("/")) ||
      (await cache.match("/offline.html")) ||
      Response.error()
    );
  }
}

async function handleAsset(request) {
  const cache = await caches.open(await findActiveShellCache());
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
      const active = await findActiveShellCache();
      for (const name of await caches.keys()) {
        const managed = name.startsWith(CACHE_PREFIX) || name.startsWith("zoo-offline-");
        if (managed && name !== active) await caches.delete(name);
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
      const cache = await caches.open(await findActiveShellCache());
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
