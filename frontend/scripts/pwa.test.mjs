import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import assert from "node:assert/strict";
import { test } from "node:test";

const source = readFileSync(new URL("../public/sw.js", import.meta.url), "utf8");
function worker(overrides = {}) {
  const handlers = new Map();
  const notifications = [];
  const context = {
    URL,
    Response,
    self: {
      location: { origin: "https://zoo.example" },
      navigator: { language: "ja" },
      addEventListener: (type, handler) => handlers.set(type, handler),
      registration: { showNotification: async (...args) => notifications.push(args) },
      clients: { matchAll: async () => [], openWindow: async () => {} },
    },
    caches: { match: async () => new Response("offline") },
    fetch: async () => {
      throw new Error("offline");
    },
    ...overrides,
  };
  runInNewContext(source, context);
  return { handlers, notifications, context };
}

test("empty Web Push displays an alert without parsing payload or fetching private data", async () => {
  const { handlers, notifications } = worker();
  let work;
  handlers.get("push")({
    waitUntil: (promise) => {
      work = promise;
    },
  });
  await work;
  assert.equal(notifications.length, 1);
  assert.equal(notifications[0][0], "Zoo");
  assert.match(notifications[0][1].body, /新しい通知/);
});

test("offline navigation falls back while APIs, auth and cross-origin requests pass through", async () => {
  const { handlers } = worker();
  for (const [path, mode, expected] of [
    ["https://zoo.example/channels/123", "navigate", true],
    ["https://zoo.example/api/users/me", "navigate", false],
    ["https://zoo.example/auth/callback", "navigate", false],
    ["https://idp.example/auth", "navigate", false],
    ["https://zoo.example/api/shapes/messages", "cors", false],
  ]) {
    let response;
    handlers.get("fetch")({
      request: { url: path, method: "GET", mode },
      respondWith: (value) => {
        response = value;
      },
    });
    assert.equal(Boolean(response), expected, path);
    if (response) assert.equal(await (await response).text(), "offline");
  }
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
  await work;
  assert.equal(closed, true);
  assert.equal(focused, true);
  assert.equal(opened, false);
  context.self.clients.matchAll = async () => [];
  handlers.get("notificationclick")(event);
  await work;
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
