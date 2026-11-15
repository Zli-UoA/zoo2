import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import assert from "node:assert/strict";
import { test } from "node:test";
import { webcrypto } from "node:crypto";

const ORIGIN = "https://zoo.example";
const source = readFileSync(new URL("../public/sw.js", import.meta.url), "utf8");

function basicResponse(body, init = {}) {
  const response = new Response(body, init);
  Object.defineProperty(response, "type", { value: "basic" });
  return response;
}

function requestKey(request) {
  if (typeof request === "string") return new URL(request, ORIGIN).href;
  return request.url;
}

function memoryCaches(entries = []) {
  const stores = new Map();
  const storeFor = (name) => {
    let store = stores.get(name);
    if (!store) {
      store = new Map(
        entries.filter(([cache]) => cache === name).map(([, key, value]) => [key, value]),
      );
      stores.set(name, store);
    }
    return store;
  };
  const api = {
    async open(name) {
      const store = storeFor(name);
      return {
        async put(request, response) {
          store.set(requestKey(request), response.clone());
        },
        async match(request) {
          return store.get(requestKey(request))?.clone();
        },
        async keys() {
          return [...store.keys()].map((url) => new Request(url));
        },
        async delete(request) {
          return store.delete(requestKey(request));
        },
        async addAll(requests) {
          for (const request of requests) {
            store.set(requestKey(request), basicResponse(`asset:${requestKey(request)}`));
          }
        },
      };
    },
    async match(request) {
      const key = requestKey(request);
      for (const store of stores.values()) {
        const response = store.get(key);
        if (response) return response.clone();
      }
      return undefined;
    },
    async keys() {
      return [...stores.keys()];
    },
    async delete(name) {
      return stores.delete(name);
    },
  };
  return { api, stores, storeFor };
}

function worker(overrides = {}) {
  const handlers = new Map();
  const notifications = [];
  const cacheContext = overrides.caches ?? memoryCaches();
  const baseFetch = async (input) => {
    const key = requestKey(input);
    return basicResponse(`network:${key}`);
  };
  const context = {
    crypto: webcrypto,
    AbortSignal,
    URL,
    Response,
    Headers,
    Request,
    self: {
      location: {
        origin: ORIGIN,
        href: `${ORIGIN}/sw.js${overrides.policyVersion ? `?v=${overrides.policyVersion}` : ""}`,
      },
      navigator: { language: "ja" },
      addEventListener: (type, handler) => handlers.set(type, handler),
      registration: { showNotification: async (...args) => notifications.push(args) },
      clients: {
        matchAll: async () => [],
        openWindow: async () => {},
        claim: async () => {},
      },
      skipWaiting: async () => {},
    },
    caches: cacheContext.api ?? cacheContext,
    fetch: overrides.fetch ?? baseFetch,
    ...overrides,
  };
  runInNewContext(source, context);
  return { handlers, notifications, context, cacheContext };
}

function dispatch(handlers, type, event = {}) {
  let work;
  handlers.get(type)({
    ...event,
    waitUntil: (promise) => {
      work = promise;
    },
  });
  return Promise.resolve(work);
}

async function fetchThrough(handlers, request, background = []) {
  let response;
  handlers.get("fetch")({
    request,
    waitUntil: (promise) => {
      background.push(promise);
    },
    respondWith: (value) => {
      response = value;
    },
  });
  return response === undefined ? undefined : await Promise.resolve(response);
}

function navigation(url, cache = "default") {
  return { url, method: "GET", mode: "navigate", cache };
}

test("legacy empty Web Push still displays a generic alert", async () => {
  const { handlers, notifications } = worker();
  await dispatch(handlers, "push");
  assert.equal(notifications.length, 1);
  assert.equal(notifications[0][0], "Zoo");
  assert.match(notifications[0][1].body, /新しい通知/);
  assert.equal(notifications[0][1].icon, "/icons/icon-192.png");
  assert.equal(notifications[0][1].badge, "/icons/badge-96.png");
});

test("install precaches the app shell so offline navigation can start the SPA", async () => {
  const { handlers, cacheContext } = worker();
  await dispatch(handlers, "install");
  const shell = cacheContext.storeFor("zoo-shell-v2");
  for (const path of [
    "/",
    "/index.html",
    "/offline.html",
    "/manifest.webmanifest",
    "/favicon.ico",
    "/icons/zoo.svg",
    "/icons/favicon-32.png",
    "/icons/badge-96.png",
  ]) {
    assert.ok(shell.has(`${ORIGIN}${path}`), path);
  }
  assert.match(await shell.get(`${ORIGIN}/index.html`).text(), /network:/);
});

test("install precaches the hashed assets referenced by the app shell", async () => {
  const { handlers, cacheContext } = worker({
    fetch: async (input) => {
      const key = requestKey(input);
      if (key === `${ORIGIN}/` || key === `${ORIGIN}/index.html`) {
        return basicResponse(
          '<html><head><link rel="stylesheet" href="/assets/app.css"></head><body><script type="module" src="/assets/app.js"></script></body></html>',
          { headers: { "content-type": "text/html" } },
        );
      }
      return basicResponse(`network:${key}`);
    },
  });
  await dispatch(handlers, "install");
  const shell = cacheContext.storeFor("zoo-shell-v2");
  assert.ok(shell.has(`${ORIGIN}/assets/app.css`));
  assert.ok(shell.has(`${ORIGIN}/assets/app.js`));
});

test("failed install entry assets preserve the previous shell and omit the new marker", async () => {
  const cacheContext = memoryCaches([
    ["zoo-shell-old", `${ORIGIN}/index.html`, basicResponse("working-shell")],
  ]);
  const { handlers } = worker({
    caches: cacheContext.api,
    fetch: async (request) => {
      if (requestKey(request).includes("/assets/"))
        return new Response("unavailable", { status: 503 });
      return basicResponse('<script src="/assets/index-new.js"></script>');
    },
  });
  await assert.rejects(dispatch(handlers, "install"), /without its entry assets/);
  const failed = cacheContext.storeFor("zoo-shell-new");
  assert.equal(failed.has(`${ORIGIN}/__zoo_shell_meta__`), false);
  assert.equal(failed.has(`${ORIGIN}/index.html`), false);
  assert.equal(
    await cacheContext.storeFor("zoo-shell-old").get(`${ORIGIN}/index.html`).text(),
    "working-shell",
  );
});

test("cached responses are stored without transport encoding or validators", async () => {
  const { handlers, cacheContext } = worker({
    fetch: async (input) => {
      const response = basicResponse(`body:${requestKey(input)}`, {
        headers: {
          "content-encoding": "gzip",
          "cache-control": "no-cache",
          etag: '"abc"',
          "content-type": "text/javascript",
        },
      });
      return response;
    },
  });
  await dispatch(handlers, "install");
  const shell = cacheContext.storeFor("zoo-shell-v2");
  const cached = shell.get(`${ORIGIN}/index.html`);
  assert.equal(cached.headers.get("content-encoding"), null);
  assert.equal(cached.headers.get("etag"), null);
  assert.equal(cached.headers.get("cache-control"), "no-store");
  assert.equal(cached.headers.get("content-type"), "text/javascript");
});

test("page-driven asset caching only accepts same-origin /assets paths", async () => {
  const { handlers, cacheContext } = worker();
  await dispatch(handlers, "message", {
    data: {
      type: "CACHE_ASSETS",
      urls: ["/assets/lazy-worker.js", "https://evil.example/assets/x.js", "/api/users/me"],
    },
  });
  const shell = cacheContext.storeFor("zoo-shell-v2");
  assert.ok(shell.has(`${ORIGIN}/assets/lazy-worker.js`));
  assert.ok(!shell.has("https://evil.example/assets/x.js"));
  assert.ok(!shell.has(`${ORIGIN}/api/users/me`));
});

test("a restarted Service Worker still finds the build-specific shell cache", async () => {
  const first = worker();
  await dispatch(first.handlers, "install");
  // A new worker instance has an empty global scope, like Chrome after ~30s.
  const restarted = worker({
    caches: first.cacheContext.api,
    fetch: async () => {
      throw new Error("offline");
    },
  });
  const response = await fetchThrough(restarted.handlers, navigation(`${ORIGIN}/channels/123`));
  assert.match(await response.text(), /network:https:\/\/zoo\.example\/index\.html/);
});

test("activation removes older cache versions and claims clients", async () => {
  const { handlers, context, cacheContext } = worker();
  let claimed = false;
  context.self.clients.claim = async () => {
    claimed = true;
  };
  await cacheContext.api.open("zoo-offline-v1");
  await cacheContext.api.open("unrelated-cache");
  await dispatch(handlers, "activate");
  const names = await cacheContext.api.keys();
  assert.ok(!names.includes("zoo-offline-v1"));
  assert.ok(names.includes("unrelated-cache"));
  assert.equal(claimed, true);
});

test("offline navigation falls back to the cached app shell, then offline.html", async () => {
  const { handlers, cacheContext } = worker({
    fetch: async (input) => {
      if (requestKey(input).includes("/channels/")) throw new Error("offline");
      return basicResponse(`network:${requestKey(input)}`);
    },
  });
  await dispatch(handlers, "install");
  const shell = cacheContext.storeFor("zoo-shell-v2");
  // Installation cached the offline page; without an app shell it is the last resort.
  shell.delete(`${ORIGIN}/index.html`);
  shell.delete(`${ORIGIN}/`);
  let response = await fetchThrough(handlers, navigation(`${ORIGIN}/channels/123`));
  assert.equal(await response.text(), "network:https://zoo.example/offline.html");

  shell.set(
    `${ORIGIN}/index.html`,
    basicResponse("cached-shell", { headers: { "content-type": "text/html" } }),
  );
  response = await fetchThrough(handlers, navigation(`${ORIGIN}/channels/456`));
  assert.equal(await response.text(), "cached-shell");
});

test("online navigation stores the static shell for later deep-link reloads", async () => {
  const { handlers, cacheContext } = worker({
    fetch: async (input) =>
      basicResponse(`html:${requestKey(input)}`, { headers: { "content-type": "text/html" } }),
  });
  const response = await fetchThrough(handlers, navigation(`${ORIGIN}/channels/123`));
  assert.match(await response.text(), /html:/);
  const shell = cacheContext.storeFor("zoo-shell-v2");
  assert.ok(shell.has(`${ORIGIN}/index.html`));
});

test(
  "a cached PWA starts before a slow navigation refresh finishes",
  { timeout: 1000 },
  async () => {
    const cacheContext = memoryCaches([
      ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
    ]);
    let release;
    const pending = new Promise((resolve) => {
      release = resolve;
    });
    const { handlers } = worker({
      caches: cacheContext.api,
      fetch: async (request) => {
        if (requestKey(request).includes("/pwa-assets.json"))
          return Response.json({
            entry: "/assets/index-new.js",
            assets: ["/assets/index-new.js", "/assets/opfs-new.js"],
          });
        if (requestKey(request).includes("/assets/")) return basicResponse("updated-code");
        await pending;
        return basicResponse('<script src="/assets/index-new.js"></script>updated-shell', {
          headers: { "content-type": "text/html" },
        });
      },
    });
    const background = [];
    const response = await fetchThrough(
      handlers,
      navigation(`${ORIGIN}/channels/general`),
      background,
    );
    assert.equal(await response.text(), "saved-shell");
    release();
    await Promise.all(background);
    const cache = cacheContext.storeFor("zoo-shell-v2");
    assert.match(await cache.get(`${ORIGIN}/index.html`).text(), /updated-shell/);
    assert.ok(cache.has(`${ORIGIN}/assets/index-new.js`));
    assert.ok(cache.has(`${ORIGIN}/assets/opfs-new.js`));
  },
);

for (const cache of ["no-cache", "reload"]) {
  test(`explicit ${cache} navigation fetches the deployed shell and still works offline`, async () => {
    const cacheContext = memoryCaches([
      ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
    ]);
    let offline = false;
    const { handlers } = worker({
      caches: cacheContext.api,
      fetch: async (request) => {
        if (offline) throw new Error("offline");
        if (requestKey(request).includes("/pwa-assets.json"))
          return Response.json({
            entry: "/assets/index-reloaded.js",
            assets: ["/assets/index-reloaded.js", "/assets/lazy-reloaded.js"],
          });
        if (requestKey(request).includes("/assets/")) return basicResponse("code");
        return basicResponse('<script src="/assets/index-reloaded.js"></script>deployed-shell', {
          headers: { "content-type": "text/html" },
        });
      },
    });
    const background = [];
    const request = navigation(`${ORIGIN}/channels/general`, cache);
    assert.match(
      await (await fetchThrough(handlers, request, background)).text(),
      /deployed-shell/,
    );
    assert.equal(background.length, 0);
    offline = true;
    assert.match(await (await fetchThrough(handlers, request)).text(), /deployed-shell/);
    assert.ok(cacheContext.storeFor("zoo-shell-v2").has(`${ORIGIN}/assets/lazy-reloaded.js`));
  });
}

for (const path of [
  "/licenses/README.txt",
  "/licenses/misskey-emoji-data/LICENSE",
  "/pwa-assets.json",
  "/mfm-emojis.json",
  "/compose-catalog.html",
]) {
  test(`direct navigation to ${path} keeps the static response`, async () => {
    const cacheContext = memoryCaches([
      ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
    ]);
    const { handlers } = worker({
      caches: cacheContext.api,
      fetch: async () => basicResponse("static-file", { headers: { "content-type": "text/html" } }),
    });
    assert.equal(
      await (await fetchThrough(handlers, navigation(`${ORIGIN}${path}`))).text(),
      "static-file",
    );
    assert.equal(
      await cacheContext.storeFor("zoo-shell-v2").get(`${ORIGIN}/index.html`).text(),
      "saved-shell",
    );
  });
}

test("a failed entry asset refresh keeps the working offline shell", async () => {
  const cacheContext = memoryCaches([
    ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
  ]);
  const { handlers } = worker({
    caches: cacheContext.api,
    fetch: async (request) => {
      if (requestKey(request).includes("/assets/")) throw new Error("asset unavailable");
      return basicResponse('<script src="/assets/missing.js"></script>new-shell', {
        headers: { "content-type": "text/html" },
      });
    },
  });
  const background = [];
  assert.equal(
    await (
      await fetchThrough(handlers, navigation(`${ORIGIN}/channels/general`), background)
    ).text(),
    "saved-shell",
  );
  await Promise.all(background);
  assert.equal(
    await cacheContext.storeFor("zoo-shell-v2").get(`${ORIGIN}/index.html`).text(),
    "saved-shell",
  );
});

for (const failure of ["lazy-asset", "mismatched-build"]) {
  test(`a ${failure} failure keeps the previous complete shell`, async () => {
    const cacheContext = memoryCaches([
      ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
    ]);
    const { handlers } = worker({
      caches: cacheContext.api,
      fetch: async (request) => {
        const path = new URL(requestKey(request)).pathname;
        if (path === "/pwa-assets.json")
          return Response.json({
            entry:
              failure === "mismatched-build" ? "/assets/index-other.js" : "/assets/index-new.js",
            assets: ["/assets/index-new.js", "/assets/opfs-new.js"],
          });
        if (path === "/assets/opfs-new.js") throw new Error("missing lazy worker");
        if (path.startsWith("/assets/")) return basicResponse("code");
        return basicResponse('<script src="/assets/index-new.js"></script>', {
          headers: { "content-type": "text/html" },
        });
      },
    });
    const background = [];
    await fetchThrough(handlers, navigation(`${ORIGIN}/`), background);
    await Promise.all(background);
    assert.equal(
      await cacheContext.storeFor("zoo-shell-v2").get(`${ORIGIN}/index.html`).text(),
      "saved-shell",
    );
  });
}

test("the active worker reports the entry used by its installed shell", async () => {
  const cacheContext = memoryCaches([
    [
      "zoo-shell-v2",
      `${ORIGIN}/index.html`,
      basicResponse('<script src="/assets/index-updated.js"></script>'),
    ],
  ]);
  const { handlers } = worker({ caches: cacheContext.api });
  let reply;
  await dispatch(handlers, "message", {
    data: { type: "SHELL_ENTRY" },
    ports: [
      {
        postMessage: (value) => {
          reply = value;
        },
      },
    ],
  });
  assert.equal(reply.entry, "/assets/index-updated.js");
});

test("direct navigation to a non-HTML asset does not overwrite the app shell", async () => {
  const { handlers, cacheContext } = worker({
    fetch: async (input) =>
      basicResponse(`png:${requestKey(input)}`, { headers: { "content-type": "image/png" } }),
  });
  const shell = cacheContext.storeFor("zoo-shell-v2");
  shell.set(`${ORIGIN}/index.html`, basicResponse("cached-shell"));
  const response = await fetchThrough(handlers, navigation(`${ORIGIN}/icons/icon-512.png`));
  assert.match(await response.text(), /png:/);
  assert.equal(await shell.get(`${ORIGIN}/index.html`).text(), "cached-shell");
});

test("a build-specific cache name replaces the previous shell cache", async () => {
  const indexHtml =
    '<html><head><script type="module" src="/assets/index-abc123.js"></script></head></html>';
  const { handlers, cacheContext } = worker({
    fetch: async (input) => {
      const key = requestKey(input);
      if (key === `${ORIGIN}/` || key === `${ORIGIN}/index.html`) {
        return basicResponse(indexHtml, { headers: { "content-type": "text/html" } });
      }
      return basicResponse(`network:${key}`);
    },
  });
  await dispatch(handlers, "install");
  const names = await cacheContext.api.keys();
  assert.ok(names.includes("zoo-shell-abc123"));
  assert.ok(!names.includes("zoo-shell-v2"));

  await cacheContext.api.open("zoo-shell-v2");
  await dispatch(handlers, "activate");
  const after = await cacheContext.api.keys();
  assert.ok(after.includes("zoo-shell-abc123"));
  assert.ok(!after.includes("zoo-shell-v2"));
});

test("APIs, auth, cross-origin requests and non-GET requests pass through", async () => {
  const { handlers } = worker();
  for (const [url, mode, method] of [
    [`${ORIGIN}/api/users/me`, "navigate", "GET"],
    [`${ORIGIN}/auth/callback`, "navigate", "GET"],
    [`${ORIGIN}/android/reauth-callback?code=c&state=s`, "navigate", "GET"],
    [`${ORIGIN}/api/shapes/messages`, "cors", "GET"],
    ["https://idp.example/auth", "navigate", "GET"],
    [`${ORIGIN}/assets/app.js`, "cors", "POST"],
  ]) {
    const response = await fetchThrough(handlers, { url, mode, method });
    assert.equal(response, undefined, `${method} ${url}`);
  }
});

test("static assets are cached on first fetch and served from the cache afterwards", async () => {
  let requests = 0;
  const { handlers } = worker({
    fetch: async (input) => {
      requests += 1;
      return basicResponse(`asset-body:${requestKey(input)}`);
    },
  });
  const asset = { url: `${ORIGIN}/assets/app-abc.js`, method: "GET", mode: "cors" };
  const first = await fetchThrough(handlers, asset);
  assert.match(await first.text(), /asset-body:/);
  const second = await fetchThrough(handlers, asset);
  assert.match(await second.text(), /asset-body:/);
  assert.equal(requests, 1);
});

test("the E2EE wasm is precached with the build and served offline as application/wasm", async () => {
  // crates/zoo-crypto-wasm の *_bg.wasm は Vite が assets/ に出し、pwa-assets.json に入る（#352）。
  // ページが読む前に保存しておけば、オフラインでも Worker が wasm を読める。
  const wasm = "/assets/zoo_crypto_wasm_bg-abc.wasm";
  const cacheContext = memoryCaches([
    ["zoo-shell-v2", `${ORIGIN}/index.html`, basicResponse("saved-shell")],
  ]);
  let offline = false;
  const { handlers } = worker({
    caches: cacheContext.api,
    fetch: async (request) => {
      if (offline) throw new Error("offline");
      const path = new URL(requestKey(request)).pathname;
      if (path === "/pwa-assets.json")
        return Response.json({
          entry: "/assets/index-new.js",
          assets: ["/assets/index-new.js", wasm],
        });
      if (path === wasm)
        return basicResponse("wasm-bytes", { headers: { "content-type": "application/wasm" } });
      if (path.startsWith("/assets/")) return basicResponse("code");
      return basicResponse('<script src="/assets/index-new.js"></script>new-shell', {
        headers: { "content-type": "text/html" },
      });
    },
  });
  await fetchThrough(handlers, navigation(`${ORIGIN}/`, "reload"));
  assert.ok(cacheContext.storeFor("zoo-shell-v2").has(`${ORIGIN}${wasm}`));
  offline = true;
  const response = await fetchThrough(handlers, {
    url: `${ORIGIN}${wasm}`,
    method: "GET",
    mode: "cors",
  });
  assert.equal(await response.text(), "wasm-bytes");
  assert.equal(response.headers.get("content-type"), "application/wasm");
});

test("notification click focuses an existing app or opens the app root", async () => {
  const { handlers, context } = worker();
  let focused = false,
    opened = false,
    closed = false,
    work;
  context.self.clients.matchAll = async () => [
    {
      url: "https://zoo.example/channels/1",
      focus: async () => {
        focused = true;
      },
    },
  ];
  context.self.clients.openWindow = async (url) => {
    assert.equal(url, "/");
    opened = true;
  };
  const event = {
    notification: {
      close: () => {
        closed = true;
      },
    },
    waitUntil: (value) => {
      work = value;
    },
  };
  handlers.get("notificationclick")(event);
  await Promise.resolve(work);
  assert.equal(closed, true);
  assert.equal(focused, true);
  assert.equal(opened, false);
  context.self.clients.matchAll = async () => [];
  handlers.get("notificationclick")(event);
  await Promise.resolve(work);
  assert.equal(opened, true);
});

test("manifest icon files exist with the declared PNG dimensions", () => {
  const manifest = JSON.parse(
    readFileSync(new URL("../public/manifest.webmanifest", import.meta.url), "utf8"),
  );
  assert.equal(manifest.display, "standalone");
  for (const icon of manifest.icons) {
    const bytes = readFileSync(new URL(`../public${icon.src}`, import.meta.url));
    const [width, height] = icon.sizes.split("x").map(Number);
    assert.equal(bytes.readUInt32BE(16), width);
    assert.equal(bytes.readUInt32BE(20), height);
  }
});

test("push displays sender, channel and message preview from the decrypted payload", async () => {
  const { handlers, notifications } = worker();
  await dispatch(handlers, "push", {
    data: {
      json: () => ({
        title: "Alice · #general",
        body: "こんにちは、Bob!",
        url: "/channels/123?message=456",
        tag: "zoo-123",
        notificationType: "mention",
      }),
    },
  });
  assert.equal(notifications[0][0], "Alice · #general");
  assert.equal(notifications[0][1].body, "こんにちは、Bob!");
  assert.equal(notifications[0][1].data.url, "/channels/123?message=456");
  assert.equal(notifications[0][1].tag, "zoo-123");
});

test("reaction notifications describe the event and malformed payloads retain the fallback", async () => {
  const { handlers, notifications } = worker();
  for (const data of [
    { json: () => ({ title: "Bob", body: "Hello", notificationType: "reaction" }) },
    {
      json: () => {
        throw new Error("bad JSON");
      },
    },
  ]) {
    await dispatch(handlers, "push", { data });
  }
  assert.equal(notifications[0][1].body, "リアクション: Hello");
  assert.equal(notifications[1][0], "Zoo");
  assert.match(notifications[1][1].body, /新しい通知/);
});

test("encrypted pushes show the body decrypted by /sw-push.js and fall back to the generic body", async () => {
  const imported = [];
  const received = [];
  const outcomes = ["秘密のこんにちは", null, new Error("store failure"), "👍"];
  const { handlers, notifications, context } = worker({
    importScripts: (url) => imported.push(url),
  });
  assert.deepEqual(imported, ["/sw-push.js"]);
  // /sw-push.js が置く関数
  context.self.zooDecryptPush = async (e2ee) => {
    received.push(e2ee);
    const outcome = outcomes.shift();
    if (outcome instanceof Error) throw outcome;
    return outcome;
  };
  const e2ee = { recipient_id: "r", message: { channel_id: "c" } };
  const push = (extra = {}) =>
    dispatch(handlers, "push", {
      data: {
        json: () => ({ title: "Alice · #secret", body: "新しいメッセージ", e2ee, ...extra }),
      },
    });
  await push();
  await push();
  await push();
  await push({ notificationType: "reaction" });
  // e2ee のない Push は復号しない
  await dispatch(handlers, "push", {
    data: { json: () => ({ title: "Bob", body: "plain" }) },
  });
  assert.equal(received.length, 4);
  assert.deepEqual(received[0], e2ee);
  assert.deepEqual(
    notifications.map(([, options]) => options.body),
    ["秘密のこんにちは", "新しいメッセージ", "新しいメッセージ", "リアクション: 👍", "plain"],
  );
});

test("a push decryption script that cannot be loaded fails the install so it is retried", () => {
  // importScripts の失敗を握りつぶすと、復号できない Service Worker が残る。
  assert.throws(
    () =>
      worker({
        importScripts: () => {
          throw new Error("NetworkError");
        },
      }),
    /NetworkError/,
  );
});

test("the dev placeholder for /sw-push.js leaves pushes undecrypted", async () => {
  const placeholder = readFileSync(new URL("../public/sw-push.js", import.meta.url), "utf8");
  const placeholderSelf = {};
  const { handlers, notifications, context } = worker({
    importScripts: () => runInNewContext(placeholder, { self: placeholderSelf }),
  });
  assert.deepEqual(placeholderSelf, { zooDecryptPush: undefined });
  assert.equal(context.self.zooDecryptPush, undefined);
  await dispatch(handlers, "push", {
    data: { json: () => ({ title: "Alice", body: "新しいメッセージ", e2ee: {} }) },
  });
  assert.equal(notifications[0][1].body, "新しいメッセージ");
});

test("notification click navigates an existing app to the target message", async () => {
  const { handlers, context } = worker();
  const target = "https://zoo.example/channels/123?message=456";
  const actions = [];
  context.self.clients.matchAll = async () => [
    {
      url: "https://zoo.example/",
      navigate: async (url) => {
        actions.push(url);
      },
      focus: async () => {
        actions.push("focus");
      },
    },
  ];
  await dispatch(handlers, "notificationclick", {
    notification: { close() {}, data: { url: "/channels/123?message=456" } },
  });
  assert.deepEqual(actions, [target, "focus"]);
});

test("notification destinations stay inside the app and open directly when no window exists", async () => {
  const { handlers, context } = worker();
  const opened = [];
  context.self.clients.openWindow = async (url) => {
    opened.push(url);
  };
  for (const url of [
    "/channels/123?message=456",
    "https://evil.example/channels/1",
    "/auth/callback",
    "javascript:alert(1)",
  ]) {
    await dispatch(handlers, "notificationclick", {
      notification: { close() {}, data: { url } },
    });
  }
  assert.deepEqual(opened, ["https://zoo.example/channels/123?message=456", "/", "/", "/"]);
});

test("a failed shell download aborts installation instead of replacing the working cache", async () => {
  const { handlers } = worker({
    fetch: async () => {
      throw new Error("offline");
    },
  });
  await assert.rejects(dispatch(handlers, "install"), /without a fresh index/);
});

test("policy-versioned assets bypass immutable HTTP/CDN caches while preserving the requested cache key", async () => {
  const requests = [];
  const { handlers, cacheContext } = worker({
    policyVersion: "updated-policy",
    fetch: async (input, init) => {
      requests.push({ url: requestKey(input), cache: init?.cache });
      return basicResponse("fresh asset", {
        headers: { "content-security-policy": "script-src 'self' 'wasm-unsafe-eval'" },
      });
    },
  });
  const path = `${ORIGIN}/assets/opfs-worker-stable.js`;
  const response = await fetchThrough(handlers, new Request(path));
  assert.equal(requests[0].url, `${path}?__zoo_policy=updated-policy`);
  assert.equal(requests[0].cache, "reload");
  assert.equal(
    response.headers.get("content-security-policy"),
    "script-src 'self' 'wasm-unsafe-eval'",
  );
  const cache = await cacheContext.api.open("zoo-shell-v3-updated-policy-v2");
  assert.ok(await cache.match(path));
  assert.equal(await cache.match(`${path}?__zoo_policy=updated-policy`), undefined);
});

test("unversioned assets still revalidate the HTTP cache", async () => {
  const requests = [];
  const { handlers } = worker({
    fetch: async (input, init) => {
      requests.push({ url: requestKey(input), cache: init?.cache });
      return basicResponse("asset");
    },
  });
  const url = `${ORIGIN}/assets/unversioned.js`;
  await fetchThrough(handlers, new Request(url));
  assert.deepEqual(requests, [{ url, cache: "reload" }]);
});

const avatarOrigin = "https://avater.zliusercontent.com";
const avatarRequest = (path = "/avatar.png") => ({
  url: `${avatarOrigin}${path}`,
  destination: "image",
  method: "GET",
  mode: "no-cors",
});
const avatarResponse = (body = "first", init = {}) =>
  new Response(body, { ...init, headers: { "content-type": "image/png", ...init.headers } });
function avatarWorker(fetch) {
  let time = 1_000_000;
  const result = worker({
    fetch,
    Date: class extends Date {
      static now() {
        return time;
      }
    },
  });
  return {
    ...result,
    advance: () => {
      time += 5 * 60 * 1000 + 1;
    },
  };
}

test("avatar cache serves a stored public image offline and across SW restarts", async () => {
  let requests = 0;
  const first = avatarWorker(async (_url, options) => {
    requests++;
    assert.equal(options.credentials, "omit");
    assert.equal(options.mode, "cors");
    assert.equal(options.referrerPolicy, "no-referrer");
    return avatarResponse();
  });
  assert.equal(await (await fetchThrough(first.handlers, avatarRequest())).text(), "first");
  const restarted = worker({
    caches: first.cacheContext.api,
    fetch: async () => {
      throw new Error("offline");
    },
  });
  const background = [];
  assert.equal(
    await (await fetchThrough(restarted.handlers, avatarRequest(), background)).text(),
    "first",
  );
  await Promise.all(background);
  assert.equal(requests, 1);
});

test("stale avatars display immediately, share a refresh, and notify changes without refetch loops", async () => {
  let requests = 0,
    finish;
  const instance = avatarWorker(async () => {
    requests++;
    if (requests === 1) return avatarResponse();
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  const messages = [];
  instance.context.self.clients.matchAll = async () => [
    { postMessage: (data) => messages.push(data) },
  ];
  await fetchThrough(instance.handlers, avatarRequest());
  instance.advance();
  const background = [];
  const images = await Promise.all(
    Array.from({ length: 8 }, () => fetchThrough(instance.handlers, avatarRequest(), background)),
  );
  for (const image of images) assert.equal(await image.text(), "first");
  assert.equal(requests, 2);
  finish(avatarResponse("changed"));
  await Promise.all(background);
  assert.equal(messages.length, 1);
  assert.equal(messages[0].type, "AVATAR_UPDATED");
  assert.equal(messages[0].url, avatarRequest().url);
  assert.match(messages[0].revision, /^[a-f0-9]{64}$/);
  const revised = avatarRequest(`/avatar.png?__zoo_avatar=${messages[0].revision}`);
  assert.equal(await (await fetchThrough(instance.handlers, revised)).text(), "changed");
  assert.equal(requests, 2);
  assert.equal(instance.cacheContext.storeFor("zoo-avatars-v1").size, 1);
});

test("unchanged avatars refresh the timestamp without notifying the page", async () => {
  let requests = 0;
  const instance = avatarWorker(async () => {
    requests++;
    return avatarResponse();
  });
  instance.context.self.clients.matchAll = async () => {
    throw new Error("unchanged bytes should not notify");
  };
  await fetchThrough(instance.handlers, avatarRequest());
  instance.advance();
  await dispatch(instance.handlers, "message", {
    data: { type: "REFRESH_AVATARS", urls: [avatarRequest().url] },
  });
  await fetchThrough(instance.handlers, avatarRequest());
  assert.equal(requests, 2);
});

test("failed, non-image and oversized avatar refreshes keep the last good image", async () => {
  let response = avatarResponse();
  const instance = avatarWorker(async () => response.clone());
  await fetchThrough(instance.handlers, avatarRequest());
  for (const bad of [
    avatarResponse("missing", { status: 404 }),
    new Response("html"),
    avatarResponse("x".repeat(2 * 1024 * 1024 + 1)),
  ]) {
    response = bad;
    instance.advance();
    const background = [];
    assert.equal(
      await (await fetchThrough(instance.handlers, avatarRequest(), background)).text(),
      "first",
    );
    await Promise.all(background);
    assert.equal(
      await instance.cacheContext
        .storeFor("zoo-avatars-v1")
        .get(avatarRequest().url)
        .clone()
        .text(),
      "first",
    );
  }
});

test("unconfigured CORS preserves native loading without caching an opaque response", async () => {
  let requests = 0;
  const instance = avatarWorker(async (request) => {
    requests++;
    if (typeof request === "string") throw new TypeError("CORS");
    return avatarResponse("native");
  });
  assert.equal(await (await fetchThrough(instance.handlers, avatarRequest())).text(), "native");
  assert.equal(requests, 2);
  assert.equal(instance.cacheContext.storeFor("zoo-avatars-v1").size, 0);
});

test("avatar storage is bounded and survives app shell upgrades", async () => {
  const instance = avatarWorker(async () => avatarResponse());
  for (let index = 0; index < 66; index++)
    await fetchThrough(instance.handlers, avatarRequest(`/image-${index}.png`));
  const cache = instance.cacheContext.storeFor("zoo-avatars-v1");
  assert.equal(cache.size, 64);
  assert.ok(!cache.has(`${avatarOrigin}/image-0.png`));
  assert.ok(cache.has(`${avatarOrigin}/image-65.png`));
  await instance.cacheContext.api.open("zoo-avatars-v0");
  await dispatch(instance.handlers, "activate");
  assert.ok(instance.cacheContext.stores.has("zoo-avatars-v1"));
  assert.ok(!instance.cacheContext.stores.has("zoo-avatars-v0"));
});

test("avatar caching only handles public avatar images and validates refresh URLs", async () => {
  let requests = 0;
  const instance = avatarWorker(async () => {
    requests++;
    return avatarResponse();
  });
  for (const request of [
    { ...avatarRequest(), destination: "empty" },
    { ...avatarRequest(), method: "POST" },
    { ...avatarRequest(), url: "https://other.example/photo.png" },
    { ...avatarRequest(), url: `${ORIGIN}/api/users/me` },
  ])
    assert.equal(await fetchThrough(instance.handlers, request), undefined);
  await dispatch(instance.handlers, "message", {
    data: {
      type: "REFRESH_AVATARS",
      urls: [
        "https://other.example/image.png",
        `${ORIGIN}/api/users/me`,
        `https://user:pass@avater.zliusercontent.com/x`,
      ],
    },
  });
  assert.equal(requests, 0);
});

test("avatar cache storage failures preserve native image loading", async () => {
  const cacheContext = memoryCaches();
  const open = cacheContext.api.open.bind(cacheContext.api);
  cacheContext.api.open = async (name) => {
    if (name === "zoo-avatars-v1") throw new Error("Storage unavailable");
    return open(name);
  };
  const instance = worker({
    caches: cacheContext.api,
    fetch: async () => avatarResponse("native"),
  });
  assert.equal(await (await fetchThrough(instance.handlers, avatarRequest())).text(), "native");
});

test("avatar background work extends the fetch event during dispatch", async () => {
  const instance = avatarWorker(async () => avatarResponse());
  const responses = [],
    background = [];
  instance.handlers.get("fetch")({
    request: avatarRequest(),
    respondWith(work) {
      responses.push(work);
    },
    waitUntil(work) {
      background.push(work);
    },
  });
  assert.ok(background.length > 0);
  assert.equal(responses.length, 1);
  const [response] = await Promise.all(responses);
  assert.equal(await response.text(), "first");
  await Promise.all(background);
});
