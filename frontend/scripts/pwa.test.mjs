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

test("legacy empty Web Push still displays a generic alert", async () => {
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

test("push displays sender, channel and message preview from the decrypted payload", async () => {
  const { handlers, notifications } = worker();
  let work;
  handlers.get("push")({
    data: {
      json: () => ({
        title: "Alice · #general",
        body: "こんにちは、Bob!",
        url: "/channels/123?message=456",
        tag: "zoo-123",
        notificationType: "mention",
      }),
    },
    waitUntil: (value) => {
      work = value;
    },
  });
  await work;
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
    let work;
    handlers.get("push")({
      data,
      waitUntil: (value) => {
        work = value;
      },
    });
    await work;
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
  let work;
  handlers.get("notificationclick")({
    notification: { close() {}, data: { url: "/channels/123?message=456" } },
    waitUntil: (value) => {
      work = value;
    },
  });
  await work;
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
    let work;
    handlers.get("notificationclick")({
      notification: { close() {}, data: { url } },
      waitUntil: (value) => {
        work = value;
      },
    });
    await work;
  }
  assert.deepEqual(opened, ["https://zoo.example/channels/123?message=456", "/", "/", "/"]);
});
