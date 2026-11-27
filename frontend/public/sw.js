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
// manifest はインストール済みのアプリが配色を参照する。キャッシュ優先のままだと
// background_color の変更が古い端末に届かないため、常に取り直す。offline.html も
// 同じく最新の表示を優先する（オフライン時は precache に戻る）。
const FRESH_ASSETS = new Set(["/manifest.webmanifest", "/offline.html"]);

// 暗号化チャンネルの Push の復号（#365。docs/e2ee.md の「Web Push の復号」）。`pnpm build` が
// src/sw/push.ts から作る /sw-push.js が `self.zooDecryptPush` を置く（dev サーバーでは、何もしない
// public/sw-push.js を返す）。importScripts は起動時と install のあいだにしか読めず、読めたスクリプト
// だけが更新の検査の対象になる。そのため失敗を握りつぶさず、install を失敗させて、次の登録や更新の
// 検査で読み直させる（握りつぶすと、復号できない Service Worker が sw.js の次の変更まで残る）。
if (typeof importScripts === "function") importScripts("/sw-push.js");

// /android/ は Android アプリの E2EE の再認証のリダイレクト先（#360）。App Links でアプリに渡らなかった
// ときに nginx の案内のページを出すため、app shell を返さない。
function isPrivatePath(pathname) {
  return (
    pathname.startsWith("/api/") ||
    pathname.startsWith("/auth/") ||
    pathname.startsWith("/android/")
  );
}

function isCacheableAsset(url) {
  return (
    CACHEABLE_PREFIXES.some((prefix) => url.pathname.startsWith(prefix)) ||
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
async function findActiveShellCache(prefix = POLICY_PREFIX) {
  const names = (await caches.keys()).filter((name) => name.startsWith(prefix));
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
  return prefix === POLICY_PREFIX ? DEFAULT_CACHE : null;
}

const TRANSPORT_HEADERS = [
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
];

async function fetchBounded(input, init = {}) {
  if (typeof AbortSignal !== "undefined" && typeof AbortSignal.timeout === "function")
    return await fetch(input, { ...init, signal: AbortSignal.timeout(10_000) });
  // AbortSignal.timeout の無い Service Worker でも取得できるようにする。
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10_000);
  try {
    const response = await fetch(input, { ...init, signal: controller.signal });
    // HTTP エラーはヘッダー受信時点で呼び出し元へ渡す（本文の停止を待たない）。
    if (!response.ok) return response;
    // fetch() はヘッダー受信で解決するため、本文を読み切るまで期限を維持する。
    await response.clone().arrayBuffer();
    return response;
  } finally {
    clearTimeout(timer);
  }
}

function cacheHeaders(response, validators = false) {
  const headers = new Headers(response.headers);
  if (validators) {
    for (const name of ["etag", "last-modified"])
      if (headers.has(name)) headers.set(`x-zoo-${name}`, headers.get(name));
  }
  for (const name of TRANSPORT_HEADERS) headers.delete(name);
  // The browser may otherwise revalidate a cached response with the network.
  headers.set("cache-control", "no-store");
  return headers;
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
  const headers = cacheHeaders(response, validators);
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

async function shellCacheNamesNewestFirst(except) {
  let keys;
  try {
    keys = await caches.keys();
  } catch {
    return null;
  }
  const candidates = [];
  for (const name of keys.filter((name) => name.startsWith(CACHE_PREFIX) && name !== except)) {
    let installedAt = 0;
    try {
      const cache = await caches.open(name);
      const meta = await cache.match(META_PATH);
      const value = meta ? Number(await meta.text()) : 0;
      if (Number.isFinite(value)) installedAt = value;
    } catch {
      /* A cache whose metadata is unreadable is treated as unknown age. */
    }
    candidates.push({ name, installedAt });
  }
  candidates.sort((a, b) => b.installedAt - a.installedAt);
  return candidates.map((candidate) => candidate.name);
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
  const existed = (await caches.keys()).includes(name);
  const cache = await caches.open(name);
  try {
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
          const bounded = path === "/manifest.webmanifest" || path === "/offline.html";
          const response = bounded
            ? await fetchBounded(path, { cache: "reload" })
            : await fetch(path, { cache: "reload" });
          if (!response.ok) return;
          // nginx の try_files が欠けたファイルを index.html で返しても保存しない。
          if (path === "/offline.html") {
            if (!isFreshAssetResponse(path, response) || !(await isOfflinePage(response))) return;
          }
          if (path === "/manifest.webmanifest") {
            if (!isFreshAssetResponse(path, response) || !(await isFreshManifest(response))) return;
          }
          await cacheResponse(cache, path, response);
        } catch {
          /* Offline installs fall back to the runtime cache. */
        }
      }),
    );
    // 欠けた／不正な応答で保存できなかった分は旧シェルから引き継ぐ。新しいキャッシュが
    // 選択されたあとにオフラインになっても、案内と配色設定を読めるようにする。
    // 未解決のアセットだけを記録し、アセットごとに解消していく。
    const unresolved = new Set();
    // ファイルごとに、新しい世代から順に有効なコピーを探す。
    const sources = await shellCacheNamesNewestFirst(name);
    for (const path of ["/manifest.webmanifest", "/offline.html"]) {
      let savedHere = null;
      let destinationUnreadable = false;
      try {
        savedHere = await cache.match(path);
        if (await isUsableFreshAsset(path, savedHere)) continue;
      } catch {
        // 保存先を読めない場合は、旧キャッシュから復元を試みる。
        destinationUnreadable = true;
      }
      if (!sources) {
        unresolved.add(path);
        continue;
      }
      let readFailed = false;
      let restored = false;
      for (const source of sources) {
        try {
          const saved = await (await caches.open(source)).match(path);
          if (!(await isUsableFreshAsset(path, saved))) continue;
          // 案内ページ自身の CSP を保持したまま、転送ヘッダーだけ除いてコピーする。
          await cache.put(
            path,
            path === "/manifest.webmanifest" ? saved.clone() : await sanitizedCopy(saved.clone()),
          );
          restored = true;
          break;
        } catch {
          // 本文が読めない／保存できない場合は、旧キャッシュを消さないため記録する。
          readFailed = true;
        }
      }
      // 他に候補があるのに復元できなかった場合だけ未解決とする。
      if (
        !restored &&
        (readFailed || destinationUnreadable) &&
        (sources === null || sources.length > 0)
      )
        unresolved.add(path);
    }
    // 初期版の SW は案内ページを zoo-offline-v1 に持っていた。シェルに無ければ引き継ぐ。
    const hasOffline = await isUsableFreshAsset(
      "/offline.html",
      await cache.match("/offline.html").catch(() => null),
    ).catch(() => false);
    if (!hasOffline) {
      let legacyKeys = [];
      try {
        legacyKeys = await caches.keys();
      } catch {
        unresolved.add("/offline.html");
      }
      if (legacyKeys.includes("zoo-offline-v1")) {
        try {
          const saved = await (await caches.open("zoo-offline-v1")).match("/offline.html");
          if (await isUsableFreshAsset("/offline.html", saved)) {
            await cache.put("/offline.html", await sanitizedCopy(saved.clone()));
            unresolved.delete("/offline.html");
          }
        } catch {
          unresolved.add("/offline.html");
        }
      }
    }
    if (unresolved.size) throw new Error("Cannot carry over a cached app asset");
    // Write the marker last so a partially precached cache is not selected.
    await cache.put(META_PATH, new Response(String(Date.now())));
  } catch (error) {
    // 今回作成した途中のキャッシュを残すと、次回も容量不足で更新できない。
    if (!existed) await caches.delete(name).catch(() => {});
    throw error;
  }
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
    /^\/(?:channels|home|settings|notifications|catch-up|bubble|apps|apps-authorize|realtime|public\/users)(?:\/|$)/.test(
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

// nginx の try_files は見つからないファイルに index.html を返す。HTML を manifest として
// 保存するとオフライン起動の配色まで壊れるため拒否する。本番の nginx は webmanifest を
// application/octet-stream で配信するため、JSON 系だけを許可することはできない。
function isFreshAssetResponse(path, response) {
  const type = (response.headers.get("content-type") || "").toLowerCase();
  // offline.html は HTML そのものなので、こちらは HTML を要求する。
  return path === "/offline.html" ? type.includes("text/html") : !type.includes("text/html");
}

async function isFreshManifest(response) {
  // 本文が読めない場合は解析失敗と区別するため、読み取りエラーをそのまま送出する。
  const text = await response.clone().text();
  try {
    const parsed = JSON.parse(text);
    return (
      parsed !== null &&
      typeof parsed === "object" &&
      !Array.isArray(parsed) &&
      (typeof parsed.name === "string" || typeof parsed.short_name === "string") &&
      typeof parsed.start_url === "string" &&
      Array.isArray(parsed.icons) &&
      parsed.icons.length > 0
    );
  } catch {
    return false;
  }
}

async function isOfflinePage(response) {
  const text = await response.clone().text();
  // 旧形式の案内ページにもマーカーはない。title の閉じタグは改行で split されていたため空白を許容する。
  return text.includes('name="zoo-offline"') || /<title>\s*Zoo — Offline\s*<\/title\s*>/.test(text);
}

async function isUsableFreshAsset(path, response) {
  if (!response) return false;
  return (
    isFreshAssetResponse(path, response) &&
    (path !== "/manifest.webmanifest" || (await isFreshManifest(response))) &&
    (path !== "/offline.html" || (await isOfflinePage(response)))
  );
}

async function sanitizedCopy(saved) {
  return new Response(await saved.arrayBuffer(), {
    status: saved.status,
    statusText: saved.statusText,
    headers: cacheHeaders(saved),
  });
}

async function fetchConditionalAsset(request, path) {
  // 条件付き GET は共有もキャッシュ更新もしない。304 はそのまま返し、
  // 通信失敗や 5xx のときだけ保存済みの版を使う。
  try {
    const response = await fetchBounded(request, { cache: "reload" });
    if (response.status === 304 || response.ok) return response;
    const cached = await cachedFreshAsset(path);
    return cached.type === "error" ? response : cached;
  } catch {
    return await cachedFreshAsset(path);
  }
}

const freshAssetWork = new Map();

function handleFreshAsset(request) {
  const path = new URL(request.url, self.location.origin).pathname;
  // Range / 条件付き GET は部分応答・304 になり得るので、共有もキャッシュ更新もしない。
  const headers = request.headers;
  if (
    headers?.get("range") ||
    headers?.get("if-none-match") ||
    headers?.get("if-modified-since") ||
    headers?.get("if-match") ||
    headers?.get("if-unmodified-since")
  ) {
    if (headers.get("range")) return fetch(request, { cache: "reload" });
    return fetchConditionalAsset(request, path);
  }
  // 同じパスの同時取得を共有し、古い応答が新しい配色を上書きしないようにする。
  const pending = freshAssetWork.get(path);
  // 取得は共有しても、respondWith へ渡す本文は要求ごとに独立させる。
  if (pending) return pending.then((response) => response.clone());
  const work = fetchFreshAsset(request, path).finally(() => freshAssetWork.delete(path));
  freshAssetWork.set(path, work);
  return work.then((response) => response.clone());
}

async function fetchFreshAsset(request, path) {
  let response;
  try {
    // サーバーが応答を返さない／本文が途中で止まる場合も、保存済みの版に戻れるようにする。
    response = await fetchBounded(request, { cache: "reload" });
  } catch {
    return await cachedFreshAsset(path);
  }
  let acceptable = false;
  try {
    acceptable = response.ok && (await isUsableFreshAsset(path, response));
  } catch {
    return await cachedFreshAsset(path, response.ok ? null : response);
  }
  if (acceptable) {
    if (response.type !== "basic") return response;
    // 取得した案内ページ自身の CSP を保持するため、シェルの CSP では上書きしない。
    // 保存に失敗しても取得できた最新の応答は返す。古いキャッシュへ戻さない。
    const fresh = response.clone();
    try {
      const cache = await caches.open(await findActiveShellCache());
      // クエリ付きで開いても同じ案内／manifest を使うため、pathname をキーにする。
      await cacheResponse(cache, path, response);
    } catch {
      /* Storage quota or Cache API failure: still serve the fresh response. */
    }
    return fresh;
  }
  return await cachedFreshAsset(path, response.ok ? null : response);
}

async function cachedFreshAsset(path, networkResponse = null) {
  // 現行のポリシーのキャッシュを優先する。案内ページ自身の CSP はそのまま返す。
  let names = [];
  try {
    names = (await caches.keys()).filter((name) => name.startsWith(CACHE_PREFIX));
  } catch {
    /* No caches can be enumerated. */
  }
  const candidates = [];
  for (const name of names) {
    let installedAt = 0;
    try {
      const cache = await caches.open(name);
      const meta = await cache.match(META_PATH);
      const value = meta ? Number(await meta.text()) : 0;
      if (Number.isFinite(value)) installedAt = value;
    } catch {
      /* An unreadable cache has an unknown age. */
    }
    candidates.push({ name, current: name.startsWith(POLICY_PREFIX), installedAt });
  }
  candidates.sort((a, b) => Number(b.current) - Number(a.current) || b.installedAt - a.installedAt);
  for (const { name } of candidates) {
    try {
      const cache = await caches.open(name);
      const saved = await cache.match(path);
      if (!(await isUsableFreshAsset(path, saved))) continue;
      return saved;
    } catch {
      /* Try the next generation when one cache cannot be read. */
    }
  }
  return networkResponse ?? Response.error();
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

  if (FRESH_ASSETS.has(url.pathname)) {
    // /offline.html を直接開いたナビゲーションもここで取り直してキャッシュを更新する。
    event.respondWith(handleFreshAsset(event.request));
  } else if (event.request.mode === "navigate") {
    event.respondWith(handleNavigation(event.request, event));
  } else if (isCacheableAsset(url)) {
    event.respondWith(handleAsset(event.request));
  }
});

const CALL_ACCEPT_CACHE = "zoo-call-accept";

// 通知の「応答」を押した印。画面（`IncomingCallDialog`）が一度だけ読んで消し、あれば応答して通話に入る。
async function markCall(kind, inviteId) {
  const cache = await caches.open(CALL_ACCEPT_CACHE);
  await cache.put(`/__call-${kind}/${inviteId}`, new Response(String(Date.now())));
}

async function closeCallInvite(tag) {
  const open = await self.registration.getNotifications({ tag });
  for (const notification of open) notification.close();
  if (open.length === 0) {
    // 通知を出さない push は、ブラウザーが代わりの通知を出すことがある。同じ tag で出してすぐ閉じる。
    await self.registration.showNotification("Zoo", { tag, silent: true });
    for (const notification of await self.registration.getNotifications({ tag })) {
      notification.close();
    }
  }
}

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
  if (payload.notificationType === "call_invite_end" && typeof payload.tag === "string") {
    // 通話の招待が終わった（応答・拒否・取り消し・不在）。出ている着信の通知を閉じる。
    event.waitUntil(closeCallInvite(payload.tag));
    return;
  }
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
      const isCallInvite =
        payload.notificationType === "call_invite" && typeof payload.inviteId === "string";
      await self.registration.showNotification(title, {
        body,
        icon: "/icons/icon-192.png",
        badge: "/icons/badge-96.png",
        tag: typeof payload.tag === "string" ? payload.tag : "zoo-activity",
        data: {
          url: typeof payload.url === "string" ? payload.url : "/",
          ...(isCallInvite ? { inviteId: payload.inviteId } : {}),
        },
        // 着信は、応答するか拒否するまで出し続ける（期限が来たら call_invite_end で閉じる）。
        ...(isCallInvite
          ? {
              requireInteraction: true,
              renotify: true,
              actions: [
                {
                  action: "accept",
                  title: language.startsWith("ja")
                    ? "応答"
                    : language.startsWith("zh")
                      ? "接聽"
                      : "Accept",
                },
                {
                  action: "decline",
                  title: language.startsWith("ja")
                    ? "拒否"
                    : language.startsWith("zh")
                      ? "拒絕"
                      : "Decline",
                },
              ],
            }
          : {}),
      });
    })(),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const inviteId = event.notification.data?.inviteId;
  event.waitUntil(
    (async () => {
      if (typeof inviteId === "string") {
        const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
        const appWindows = windows.filter(
          (client) => new URL(client.url).origin === self.location.origin,
        );
        if (event.action === "decline") {
          // Service Worker は API のトークンを持たないので、画面に拒否を頼む。
          // 開いているタブがあれば postMessage、なければ印を残して画面を開く（画面が印を読んで拒否する）。
          for (const client of appWindows) {
            client.postMessage({ type: "zoo-call-invite", action: "decline", inviteId });
          }
          if (appWindows.length === 0) {
            await markCall("decline", inviteId);
            const url = new URL(event.notification.data?.url || "/", self.location.origin);
            if (url.origin === self.location.origin && url.pathname.startsWith("/channels/")) {
              await self.clients.openWindow(url.href);
            }
          }
          return;
        }
        if (event.action === "accept") {
          // URL だけで応答させないよう、「応答を押した」印をここで残し、画面が一度だけ読んで消す。
          await markCall("accept", inviteId);
          for (const client of appWindows) {
            client.postMessage({ type: "zoo-call-invite", action: "accept", inviteId });
          }
        }
        if (appWindows.length > 0) {
          // 開いているタブは、移動（再読み込み）せずに前へ出す。通話中のタブを壊さない。
          for (const client of appWindows) {
            try {
              await client.focus();
              return;
            } catch {
              /* Try another app window or open a new one. */
            }
          }
        }
      }
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
