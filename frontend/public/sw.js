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
const POLICY_PREFIX = POLICY_VERSION ? `zoo-shell-v3-${POLICY_VERSION}-` : "zoo-shell-";
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
  "/favicon.ico",
  "/icons/zoo.svg",
  "/icons/favicon-32.png",
  "/icons/badge-96.png",
  "/icons/icon-180.png",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
];
const CACHEABLE_PREFIXES = ["/assets/", "/icons/", "/emojibase-data/"];

// 暗号化チャンネルの Push の復号（#365。docs/e2ee.md の「Web Push の復号」）。`pnpm build` が
// src/sw/push.ts から作る /sw-push.js が `self.zooDecryptPush` を置く。importScripts は起動時にしか
// 読めない。ない（dev サーバー）か読めなければ、復号せずに「新しいメッセージ」を出す。
try {
  if (typeof importScripts === "function") importScripts("/sw-push.js");
} catch {
  /* The push handler falls back to the server's generic body. */
}

function isPrivatePath(pathname) {
  return pathname.startsWith("/api/") || pathname.startsWith("/auth/");
}

function isCacheableAsset(url) {
  return (
    CACHEABLE_PREFIXES.some((prefix) => url.pathname.startsWith(prefix)) ||
    url.pathname === "/manifest.webmanifest" ||
    url.pathname === "/favicon.ico"
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
async function cacheResponse(cache, request, response, validators = false) {
  const body = await response.arrayBuffer();
  const headers = new Headers(response.headers);
  if (validators) {
    for (const name of ["etag", "last-modified"])
      if (headers.has(name)) headers.set(`x-zoo-${name}`, headers.get(name));
  }
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

// A policy update does not change hashed asset bytes. Give the HTTP cache and CDN
// a new key as well as the Cache API, otherwise immutable responses keep the old CSP.
function fetchStaticAsset(request) {
  if (!POLICY_VERSION) return fetch(request, { cache: "reload" });
  const url = new URL(typeof request === "string" ? request : request.url, self.location.origin);
  url.searchParams.set("__zoo_policy", POLICY_VERSION);
  return fetch(new Request(url.href, typeof request === "string" ? undefined : request), {
    cache: "reload",
  });
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
    return false;
  }
  const assets = new Set();
  for (const match of html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)) {
    if (match[1]) assets.add(match[1]);
  }
  return precacheAssets(cache, [...assets]);
}

async function precacheAssets(cache, assets) {
  return Promise.all(
    assets.map(async (path) => {
      try {
        if (await cache.match(path)) return true;
        const asset = await fetchStaticAsset(path);
        if (!asset.ok) return false;
        await cacheResponse(cache, path, asset);
        return true;
      } catch {
        return false;
      }
    }),
  ).then((results) => results.every(Boolean));
}

// A new document can reference lazy workers/chunks that the currently running
// build will never request. Cache the complete build before publishing that HTML.
async function precacheBuildAssets(cache, response) {
  try {
    const html = await response.clone().text();
    const entry = html.match(/\/assets\/index-[^"'?#]+\.js/)?.[0];
    if (!entry) return false;
    const manifest = await fetch(`/pwa-assets.json?entry=${encodeURIComponent(entry)}`, {
      cache: "reload",
    });
    if (!manifest.ok) return false;
    const build = await manifest.json();
    if (
      build.entry !== entry ||
      !Array.isArray(build.assets) ||
      !build.assets.includes(entry) ||
      !build.assets.every(
        (path) =>
          typeof path === "string" &&
          /^\/assets\/[^?#]+$/.test(path) &&
          new URL(path, self.location.origin).pathname === path,
      )
    )
      return false;
    return await precacheAssets(cache, build.assets);
  } catch {
    return false;
  }
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
  if (!(await precacheIndexAssets(cache, index))) {
    throw new Error("Cannot install an app shell without its entry assets");
  }
  const cached = await cacheResponse(cache, "/index.html", index);
  // Navigation fallbacks check /index.html first, but keep / for old clients.
  await cache.put("/", cached.clone());
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

async function refreshNavigation(request, background = false) {
  try {
    const response = await fetch(request);
    const contentType = response.headers.get("content-type") || "";
    if (response.ok && contentType.includes("text/html")) {
      // The SPA shell is static. Store it under /index.html so any navigation
      // can fall back to it, regardless of the deep link that was open.
      // Direct navigations to icons or JSON must not overwrite the shell.
      const cache = await caches.open(await findActiveShellCache());
      // A background deployment must include lazy assets before the next offline launch.
      const ready = background
        ? await precacheBuildAssets(cache, response)
        : await precacheIndexAssets(cache, response);
      if (ready) {
        return await cacheResponse(cache, "/index.html", response);
      }
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

async function handleNavigation(request, event) {
  const url = new URL(request.url);
  const spaRoute =
    url.pathname === "/" ||
    url.pathname === "/index.html" ||
    /^\/(?:channels|home|settings|notifications|bubble|apps|apps-authorize|realtime|public\/users)(?:\/|$)/.test(
      url.pathname,
    );
  const cache = await caches.open(await findActiveShellCache());
  // Static HTML (for example the Compose catalog) must never replace the SPA.
  if (!spaRoute) {
    return fetch(request).catch(async () => (await cache.match(request)) || Response.error());
  }
  const cached = (await cache.match("/index.html")) || (await cache.match("/"));
  // Explicit reloads reach the deployed document so vite:preloadError can recover.
  if (request.cache === "default" && cached) {
    event.waitUntil(refreshNavigation(request, true).catch(() => {}));
    return cached;
  }
  // Even an explicit reload must keep the next offline launch complete.
  return refreshNavigation(request, Boolean(cached));
}

async function handleAsset(request) {
  const cache = await caches.open(await findActiveShellCache());
  const cached = await cache.match(request);
  if (cached) return cached;
  try {
    const response = await fetchStaticAsset(request);
    if (response.ok && response.type === "basic") {
      return await cacheResponse(cache, request, response);
    }
    return response;
  } catch {
    return Response.error();
  }
}

// Public AuthZ avatars have their own bounded cache, retained across shell upgrades.
const AVATAR_ORIGIN = "https://avater.zliusercontent.com";
const AVATAR_CACHE = "zoo-avatars-v1";
const AVATAR_REFRESH_MS = 5 * 60 * 1000;
const AVATAR_MAX_BYTES = 2 * 1024 * 1024;
const AVATAR_MAX_ENTRIES = 64;
const avatarRequests = new Map();

function avatarURL(raw) {
  try {
    const url = new URL(raw);
    if (url.origin !== AVATAR_ORIGIN || url.username || url.password) return null;
    url.searchParams.delete("__zoo_avatar");
    url.hash = "";
    return url.href;
  } catch {
    return null;
  }
}

async function avatarBytes(response) {
  if (Number(response.headers.get("content-length")) > AVATAR_MAX_BYTES) return null;
  const reader = response.body?.getReader();
  if (!reader) return null;
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > AVATAR_MAX_BYTES) {
      void reader.cancel().catch(() => {});
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function refreshAvatar(url) {
  if (avatarRequests.has(url)) return avatarRequests.get(url);
  const work = (async () => {
    const cache = await caches.open(AVATAR_CACHE);
    const previous = await cache.match(url);
    if (
      previous &&
      Date.now() - Number(previous.headers.get("x-zoo-checked-at")) < AVATAR_REFRESH_MS
    )
      return previous;
    // Version the network key for the R2 CORS rollout.
    const networkURL = new URL(url);
    networkURL.searchParams.set("__zoo_avatar_cache", "1");
    const response = await fetch(networkURL.href, {
      signal: AbortSignal.timeout(10_000),
      mode: "cors",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      cache: "no-cache",
      redirect: "error",
    }).catch(() => null);
    if (
      !response?.ok ||
      !/^image\/(png|jpeg|webp|gif|avif)(;|$)/i.test(response.headers.get("content-type") || "")
    )
      return previous ?? null;
    const bytes = await avatarBytes(response).catch(() => null);
    if (!bytes?.byteLength) return previous ?? null;
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const revision = Array.from(new Uint8Array(digest), (byte) =>
      byte.toString(16).padStart(2, "0"),
    ).join("");
    const cached = new Response(bytes, {
      headers: {
        "content-type": response.headers.get("content-type"),
        "cache-control": "no-store",
        "x-zoo-checked-at": String(Date.now()),
        "x-zoo-avatar-revision": revision,
      },
    });
    await cache.put(url, cached.clone());
    const keys = await cache.keys();
    for (const key of keys.slice(0, Math.max(0, keys.length - AVATAR_MAX_ENTRIES)))
      await cache.delete(key);
    if (previous && previous.headers.get("x-zoo-avatar-revision") !== revision) {
      const clients = await self.clients.matchAll({ type: "window" });
      for (const client of clients) client.postMessage({ type: "AVATAR_UPDATED", url, revision });
    }
    return cached;
  })().finally(() => avatarRequests.delete(url));
  avatarRequests.set(url, work);
  return work;
}

async function handleAvatar(request, keepAlive) {
  const url = avatarURL(request.url);
  const cache = await caches.open(AVATAR_CACHE);
  const previous = await cache.match(url);
  const update = refreshAvatar(url).catch(() => null);
  keepAlive(update);
  if (previous) return previous;
  const response = await update;
  if (response) return response;
  // R2 CORS can roll out separately. Native image loading remains available.
  try {
    return await fetch(request);
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
        if (
          (managed && name !== active) ||
          (name.startsWith("zoo-avatars-") && name !== AVATAR_CACHE)
        )
          await caches.delete(name);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (event) => {
  const data = event.data;
  if (data?.type === "REFRESH_AVATARS" && Array.isArray(data.urls)) {
    const urls = [
      ...new Set(
        data.urls
          .filter((url) => typeof url === "string")
          .map(avatarURL)
          .filter(Boolean),
      ),
    ].slice(0, AVATAR_MAX_ENTRIES);
    event.waitUntil(
      (async () => {
        const results = await Promise.allSettled(urls.map(refreshAvatar));
        for (let index = 0; index < results.length; index++) {
          const result = results[index];
          const revision =
            result.status === "fulfilled"
              ? result.value?.headers.get("x-zoo-avatar-revision")
              : null;
          if (revision)
            event.source?.postMessage({ type: "AVATAR_UPDATED", url: urls[index], revision });
        }
      })(),
    );
    return;
  }
  if (data?.type === "SHELL_ENTRY") {
    event.waitUntil(
      (async () => {
        const cache = await caches.open(await findActiveShellCache());
        const index = await cache.match("/index.html");
        const html = index ? await index.text() : "";
        const entry = html.match(/\/assets\/index-[^"'?#]+\.js/)?.[0] ?? null;
        event.ports?.[0]?.postMessage({ entry });
      })().catch(() => event.ports?.[0]?.postMessage({ entry: null })),
    );
    return;
  }
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
          const response = await fetchStaticAsset(url);
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
  if (event.request.method !== "GET") return;
  if (event.request.destination === "image" && avatarURL(url.href)) {
    let updating = Promise.resolve();
    const response = handleAvatar(event.request, (work) => {
      updating = work;
    }).catch(() => fetch(event.request).catch(() => Response.error()));
    event.respondWith(response);
    event.waitUntil(response.then(() => updating));
    return;
  }
  if (url.origin !== self.location.origin) return;
  if (isPrivatePath(url.pathname) || url.pathname === "/sw.js") return;

  if (event.request.mode === "navigate") {
    event.respondWith(handleNavigation(event.request, event));
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
  event.waitUntil(
    (async () => {
      let body = typeof payload.body === "string" ? payload.body : fallback;
      // 暗号化チャンネルは、手元の暗号ストアで復号できたら本文に置き換える。
      if (payload.e2ee && typeof self.zooDecryptPush === "function") {
        try {
          const decrypted = await self.zooDecryptPush(payload.e2ee);
          if (typeof decrypted === "string") body = decrypted;
        } catch {
          /* Keep the generic body. */
        }
      }
      if (payload.notificationType === "reaction") {
        body =
          (language.startsWith("ja")
            ? "リアクション: "
            : language.startsWith("zh")
              ? "回應: "
              : "Reaction: ") + body;
      }
      await self.registration.showNotification(title, {
        body,
        icon: "/icons/icon-192.png",
        badge: "/icons/badge-96.png",
        tag: typeof payload.tag === "string" ? payload.tag : "zoo-activity",
        data: { url: typeof payload.url === "string" ? payload.url : "/" },
      });
    })(),
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
