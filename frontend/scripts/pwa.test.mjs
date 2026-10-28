import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import assert from "node:assert/strict";
import { test } from "node:test";

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

async function fetchThrough(handlers, request) {
  let response;
  handlers.get("fetch")({
    request,
    respondWith: (value) => {
      response = value;
    },
  });
  return response === undefined ? undefined : await Promise.resolve(response);
}

function navigation(url) {
  return { url, method: "GET", mode: "navigate" };
}

test("legacy empty Web Push still displays a generic alert", async () => {
  const { handlers, notifications } = worker();
  await dispatch(handlers, "push");
  assert.equal(notifications.length, 1);
  assert.equal(notifications[0][0], "Zoo");
  assert.match(notifications[0][1].body, /新しい通知/);
});

test("install precaches the app shell so offline navigation can start the SPA", async () => {
  const { handlers, cacheContext } = worker();
  await dispatch(handlers, "install");
  const shell = cacheContext.storeFor("zoo-shell-v2");
  for (const path of ["/", "/index.html", "/offline.html", "/manifest.webmanifest"]) {
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
