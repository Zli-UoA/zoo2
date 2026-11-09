import assert from "node:assert/strict";
import http from "node:http";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { chromium } from "@playwright/test";

const avatarOrigin = "https://avater.zliusercontent.com";
const originalWorker = await readFile(new URL("../public/sw.js", import.meta.url), "utf8");
const originalClient = stripTypeScriptTypes(
  await readFile(new URL("../src/lib/pwa/avatar-cache.ts", import.meta.url), "utf8"),
);
const pictures = [
  await readFile(new URL("../public/icons/favicon-32.png", import.meta.url)),
  await readFile(new URL("../public/icons/badge-96.png", import.meta.url)),
];
let version = 0;
let recoveryReady = false;
const imageRequests = [];
const imageServer = http.createServer((request, response) => {
  imageRequests.push({ url: request.url, cookie: request.headers.cookie });
  response.setHeader("Content-Type", "image/png");
  response.setHeader("Cache-Control", "public, max-age=31536000, immutable");
  if (!request.url.startsWith("/no-cors")) response.setHeader("Access-Control-Allow-Origin", "*");
  if (request.url.startsWith("/recover.png") && !recoveryReady) {
    response.setHeader("Content-Type", "text/plain");
    response.writeHead(404);
    response.end("not ready");
  } else response.end(pictures[version]);
});
await new Promise((resolve) => imageServer.listen(0, "127.0.0.1", resolve));
const imageOrigin = `http://127.0.0.1:${imageServer.address().port}`;
const imageURL = `${imageOrigin}/alice.png`;
const worker = originalWorker.replaceAll(avatarOrigin, imageOrigin);
const client = originalClient.replaceAll(avatarOrigin, imageOrigin);
const html =
  '<!doctype html><title>Avatar cache</title><script type="module" src="/fixture.js"></script>';
const fixture = `
import { avatarSource, subscribeAvatar } from '/avatar-cache.js';
await navigator.serviceWorker.register('/sw.js');
await navigator.serviceWorker.ready;
if (!navigator.serviceWorker.controller) await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
window.avatar = document.createElement('img');
avatar.width = avatar.height = 32;
avatar.dataset.slot = 'avatar-image';
avatar.dataset.zooAvatarSrc = ${JSON.stringify(imageURL)};
avatar.referrerPolicy = 'no-referrer';
subscribeAvatar(${JSON.stringify(imageURL)}, () => { avatar.src = avatarSource(${JSON.stringify(imageURL)}); });
avatar.src = avatarSource(${JSON.stringify(imageURL)});
document.body.append(avatar);
window.ready = true;
`;
const appServer = http.createServer((request, response) => {
  const path = new URL(request.url, "http://localhost").pathname;
  const resources = new Map([
    ["/sw.js", worker],
    ["/avatar-cache.js", client],
    ["/fixture.js", fixture],
    ["/", html],
    ["/index.html", html],
    ["/offline.html", html],
    ["/manifest.webmanifest", JSON.stringify({ name: "Avatar test", icons: [] })],
  ]);
  const body =
    path.startsWith("/icons/") || path === "/favicon.ico"
      ? pictures[0]
      : (resources.get(path) ?? null);
  response.setHeader("Content-Type", path.endsWith(".js") ? "text/javascript" : "text/html");
  response.setHeader("Cache-Control", "no-store");
  response.setHeader(
    "Content-Security-Policy",
    `default-src 'self'; script-src 'self'; connect-src 'self' ${imageOrigin}; img-src 'self' ${imageOrigin};`,
  );
  response.writeHead(body === null ? 404 : 200);
  response.end(body ?? "not found");
});
await new Promise((resolve) => appServer.listen(0, "127.0.0.1", resolve));
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}),
  args: ["--no-sandbox"],
});
try {
  const context = await browser.newContext();
  await context.addCookies([{ url: imageOrigin, name: "avatar-cookie", value: "private" }]);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => {
    errors.push(error.message);
    console.error("Page error:", error.message);
  });
  await page.goto(`http://127.0.0.1:${appServer.address().port}/`);
  await page
    .waitForFunction(() => window.ready && window.avatar.naturalWidth === 32)
    .catch(async (error) => {
      console.error(
        "State:",
        await page.evaluate(async () => ({
          ready: window.ready,
          image: window.avatar?.outerHTML,
          width: window.avatar?.naturalWidth,
          registration: (await navigator.serviceWorker.getRegistration())?.active?.state,
          keys: await caches.keys(),
        })),
        imageRequests,
      );
      throw error;
    });
  assert.equal(imageRequests.length, 1);
  assert.equal(imageRequests[0].cookie, undefined, "SW fetches omit avatar-origin cookies");
  assert.ok(imageRequests[0].url.includes("__zoo_avatar_cache=1"));
  version = 1;
  // Simulate an entry retained since an earlier browsing session, using real Cache API data.
  await page.evaluate(async (url) => {
    const cache = await caches.open("zoo-avatars-v1");
    const response = await cache.match(url);
    const headers = new Headers(response.headers);
    headers.set("x-zoo-checked-at", "1");
    await cache.put(url, new Response(await response.arrayBuffer(), { headers }));
    window.dispatchEvent(new Event("focus"));
  }, imageURL);
  await page.waitForFunction(
    () => window.avatar.naturalWidth === 96 && window.avatar.src.includes("__zoo_avatar="),
  );
  assert.equal(imageRequests.length, 2, "a changed image updates the DOM without a reload loop");
  assert.equal(
    await page.evaluate(async () => (await (await caches.open("zoo-avatars-v1")).keys()).length),
    1,
  );

  await page.evaluate(async () => {
    const changed = new Promise((resolve) =>
      navigator.serviceWorker.addEventListener("controllerchange", resolve, { once: true }),
    );
    await navigator.serviceWorker.register("/sw.js?avatar-upgrade=1", { updateViaCache: "none" });
    await changed;
  });
  await context.setOffline(true);
  await page.evaluate((url) => {
    const image = new Image();
    image.id = "offline-probe";
    image.src = `${url}?__zoo_avatar=offline-probe`;
    document.body.append(image);
  }, imageURL);
  await page.waitForFunction(() => document.getElementById("offline-probe").complete, null, {
    timeout: 10_000,
  });
  const offlineWidth = await page.locator("#offline-probe").evaluate((image) => image.naturalWidth);
  assert.equal(offlineWidth, 96, "an upgraded SW serves the retained image offline");
  assert.equal(imageRequests.length, 2);
  await context.setOffline(false);
  await page.evaluate((url) => {
    const image = new Image();
    image.id = "cors-fallback";
    image.src = url;
    document.body.append(image);
  }, `${imageOrigin}/no-cors.png`);
  await page.waitForFunction(() => document.getElementById("cors-fallback").complete, null, {
    timeout: 10_000,
  });
  const fallbackWidth = await page
    .locator("#cors-fallback")
    .evaluate((image) => image.naturalWidth);
  assert.equal(fallbackWidth, 96);
  assert.equal(
    await page.evaluate(
      async (url) => Boolean(await (await caches.open("zoo-avatars-v1")).match(url)),
      `${imageOrigin}/no-cors.png`,
    ),
    false,
  );
  await page.evaluate(async (url) => {
    const { subscribeAvatar, avatarSource } = await import("/avatar-cache.js");
    const image = new Image();
    image.id = "recovery-probe";
    image.width = image.height = 32;
    image.dataset.zooAvatarSrc = url;
    subscribeAvatar(url, () => {
      image.src = avatarSource(url);
    });
    image.src = avatarSource(url);
    document.body.append(image);
  }, `${imageOrigin}/recover.png`);
  await page.waitForFunction(() => document.getElementById("recovery-probe").complete);
  assert.equal(await page.locator("#recovery-probe").evaluate((image) => image.naturalWidth), 0);
  recoveryReady = true;
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.waitForFunction(() => document.getElementById("recovery-probe").naturalWidth === 96);
  assert.deepEqual(errors, []);
  console.log(
    "Avatar SW: CORS, cookie omission, stale refresh, DOM update, offline SW upgrade and CORS rollout fallback and failed-image recovery passed.",
  );
} finally {
  await browser.close();
  appServer.closeAllConnections();
  imageServer.closeAllConnections();
  await Promise.all([
    new Promise((resolve) => appServer.close(resolve)),
    new Promise((resolve) => imageServer.close(resolve)),
  ]);
}
