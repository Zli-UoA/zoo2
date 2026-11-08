import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import http from "node:http";
import { chromium, firefox } from "@playwright/test";

const warm = process.argv.includes("--warm");
const assets = new URL("../dist/client/assets/", import.meta.url);
const workerName = (await readdir(assets)).find((name) => /^opfs-worker-.*\.js$/.test(name));
assert.ok(workerName, "Run pnpm build before this test");
const workerSource = await readFile(new URL(workerName, assets));
const sw = await readFile(new URL("../public/sw.js", import.meta.url), "utf8");
// Simulate the already-deployed version: Cache API versioning without HTTP cache busting.
const previousSW = sw
  .replace(
    'if (!POLICY_VERSION) return fetch(request, { cache: "reload" });',
    "return fetch(request);",
  )
  .replace("`zoo-shell-v3-${POLICY_VERSION}-`", "`zoo-shell-${POLICY_VERSION}-`");
let swSource = sw;
// Serve the production registration module, without introducing a separate test implementation.
const registrationSource = stripTypeScriptTypes(
  await readFile(new URL("../src/lib/pwa/worker-registration.ts", import.meta.url), "utf8"),
);
const nginx = await readFile(new URL("../../infra/frontend-nginx.conf", import.meta.url), "utf8");
const policies = [...nginx.matchAll(/add_header Content-Security-Policy "([^"]+)" always;/g)].map(
  (match) => match[1].replace(/\$\{CSP_[A-Z0-9_]+\}/g, ""),
);
assert.equal(policies.length, 2);
for (const policy of policies)
  assert.equal(policies[0], policy, "Document and Worker policies must agree");
const oldPolicy = policies[0]
  .replace(" 'wasm-unsafe-eval'", "")
  .replace("connect-src 'self' data:", "connect-src 'self'");
let policy = policies[0];
const originalHtml =
  '<!doctype html><title>SQLite CSP test</title><script type="module" src="/assets/index-fixture.js"></script>';
let html = originalHtml;
let servedWorkerName = workerName;
const server = http.createServer((request, response) => {
  response.setHeader("Content-Security-Policy", policy);
  const path = new URL(request.url, "http://localhost").pathname;
  response.setHeader(
    "Content-Type",
    path === "/" || path === "/index.html" ? "text/html" : "text/javascript",
  );
  response.setHeader(
    "Cache-Control",
    path === `/assets/${servedWorkerName}` ? "public, max-age=31536000, immutable" : "no-store",
  );
  if (path === "/" || path === "/index.html") response.end(html);
  else if (path === "/pwa-assets.json")
    response.end(
      JSON.stringify({
        entry: html.match(/\/assets\/index-[^"'?#]+\.js/)[0],
        assets: [html.match(/\/assets\/index-[^"'?#]+\.js/)[0], `/assets/${servedWorkerName}`],
      }),
    );
  else if (path === "/sw.js") response.end(swSource);
  else if (path === "/assets/registration.js") response.end(registrationSource);
  else if (path === `/assets/${servedWorkerName}`) response.end(workerSource);
  else if (path === `/assets/${workerName}`) response.writeHead(404).end();
  else response.end("");
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
async function registerAndControl(page, legacy = false) {
  const scriptURL = await page.evaluate(async (legacy) => {
    const { registerWorker, prepareOfflineWorker } = await import("/assets/registration.js");
    if (!legacy) await prepareOfflineWorker();
    const registration = legacy
      ? await navigator.serviceWorker.register("/sw.js", { scope: "/", updateViaCache: "none" })
      : await registerWorker();
    return (
      registration.installing?.scriptURL ??
      registration.waiting?.scriptURL ??
      registration.active?.scriptURL
    );
  }, legacy);
  assert.ok(scriptURL?.includes(legacy ? "/sw.js" : "/sw.js?v="));
  await page.waitForFunction(
    (url) => navigator.serviceWorker.controller?.scriptURL === url,
    scriptURL,
  );
  return scriptURL;
}
const sqliteProbe = async (workerName) => {
  async function connect() {
    const worker = new Worker(`/assets/${workerName}`);
    let sequence = 0;
    function request(payload) {
      const requestId = String(++sequence);
      return new Promise((resolve, reject) => {
        // 初回の WASM コンパイルと OPFS 起動は、共有 CI runner の負荷に左右される。
        // SQL 実行の制限は維持し、初期化だけ余裕を持たせる。
        const timeoutMs = payload.type === "init" ? 60_000 : 8_000;
        const timeout = setTimeout(() => {
          worker.terminate();
          reject(new Error(`SQLite Worker ${payload.type} timed out after ${timeoutMs}ms`));
        }, timeoutMs);
        worker.onmessage = ({ data }) => {
          if (data.requestId !== requestId) return;
          clearTimeout(timeout);
          if (data.ok) resolve(data.rows);
          else reject(new Error(data.error));
        };
        worker.onerror = (event) => {
          clearTimeout(timeout);
          worker.terminate();
          reject(new Error(event.message));
        };
        worker.postMessage({ params: [], ...payload, requestId });
      });
    }
    try {
      await request({ type: "init", databaseName: "csp-test.sqlite", vfsName: "opfs" });
      return { worker, request };
    } catch (error) {
      // The warm-cache test deliberately starts a CSP-blocked worker. It must
      // not survive that rejected probe and interfere with the upgraded page.
      worker.terminate();
      throw error;
    }
  }
  const first = await connect();
  await first.request({ type: "execute", sql: "CREATE TABLE messages (content TEXT)" });
  await first.request({
    type: "execute",
    sql: "INSERT INTO messages VALUES (?)",
    params: ["保存確認"],
  });
  await first.request({ type: "close" });
  first.worker.terminate();
  const second = await connect();
  try {
    return await second.request({ type: "execute", sql: "SELECT content FROM messages" });
  } finally {
    await second.request({ type: "close" });
    second.worker.terminate();
  }
};
try {
  const cases = [firefox, chromium].flatMap((engine) =>
    (warm ? ["legacy", "versioned", "same-policy-update", "lazy-assets-update"] : ["cold"]).map(
      (mode) => ({
        engine,
        mode,
      }),
    ),
  );
  for (const { engine, mode } of cases) {
    html = originalHtml;
    servedWorkerName = workerName;
    console.log(`${engine.name()}: starting ${mode} CSP check`);
    const legacy = mode === "legacy";
    swSource = mode === "same-policy-update" ? previousSW : sw;
    const browser = await engine.launch();
    try {
      const context = await browser.newContext();
      const page = await context.newPage();
      const label = `${engine.name()}: ${warm ? `warm-cache ${legacy ? "legacy" : "versioned"}` : "cold-cache"}`;
      console.log(`${label}: starting SQLite CSP test`);
      page.on("pageerror", (error) => console.error(`${label}: ${error.message}`));
      page.on("console", (message) => {
        if (message.type() === "error") console.error(`${label}: ${message.text()}`);
      });
      policy = warm ? oldPolicy : policies[0];
      await page.goto(origin);
      if (warm) {
        const oldURL = await registerAndControl(page, legacy);
        await assert.rejects(page.evaluate(sqliteProbe, workerName), /Aborted|blocked by CSP/);
        await page.waitForFunction(
          async (path) => !!(await caches.match(path)),
          `/assets/${workerName}`,
        );
        assert.equal(
          await page.evaluate(
            async (path) => (await caches.match(path)).headers.get("content-security-policy"),
            `/assets/${workerName}`,
          ),
          oldPolicy,
        );
        const oldCaches = await page.evaluate(() => caches.keys());
        policy = policies[0];
        await page.reload();
        const newURL = await registerAndControl(page);
        assert.notEqual(newURL, oldURL, "CSP-only deployments must replace the SW registration");
        await page.reload();
        await registerAndControl(page);
        await page.waitForFunction(async (oldNames) => {
          const names = await caches.keys();
          return oldNames.every((name) => !names.includes(name));
        }, oldCaches);
      }
      if (mode === "same-policy-update") {
        // The previous PR updates the SW/CSP, yet immutable HTTP responses still carry old CSP.
        await assert.rejects(page.evaluate(sqliteProbe, workerName), /Aborted|blocked by CSP/);
        const previousURL = await page.evaluate(() => navigator.serviceWorker.controller.scriptURL);
        html = originalHtml.replace("index-fixture.js", "index-updated.js");
        servedWorkerName = "opfs-worker-updated.js";
        swSource = sw;
        await page.reload();
        const updatedURL = await registerAndControl(page);
        assert.equal(updatedURL, previousURL, "SW code updates must work without a CSP change");
        assert.equal(
          await page.locator('script[type="module"]').getAttribute("src"),
          "/assets/index-fixture.js",
        );
        const reloading = page.waitForNavigation({ waitUntil: "domcontentloaded" });
        await page
          .evaluate(async () => {
            const { reloadIfShellUpdated } = await import("/assets/registration.js");
            await reloadIfShellUpdated();
          })
          .catch((error) => {
            if (!error.message.includes("Execution context was destroyed")) throw error;
          });
        await reloading;
        assert.equal(
          await page.locator('script[type="module"]').getAttribute("src"),
          "/assets/index-updated.js",
        );
        await registerAndControl(page);
      }
      if (mode === "lazy-assets-update") {
        // Same SW, new build: the running old page never requests the new lazy worker.
        html = originalHtml.replace("index-fixture.js", "index-lazy.js");
        servedWorkerName = "opfs-worker-lazy.js";
        await page.reload();
        assert.equal(
          await page.locator('script[type="module"]').getAttribute("src"),
          "/assets/index-fixture.js",
        );
        await page.waitForFunction(async () => {
          const index = await caches.match("/index.html");
          return index && (await index.text()).includes("index-lazy.js");
        });
        await context.setOffline(true);
        await page.reload();
        assert.equal(
          await page.locator('script[type="module"]').getAttribute("src"),
          "/assets/index-lazy.js",
        );
      }
      const rows = await page.evaluate(sqliteProbe, servedWorkerName);
      assert.deepEqual(rows, [{ content: "保存確認" }]);
      if (warm) {
        const cachedPolicy = await page.evaluate(
          async (path) => (await caches.match(path))?.headers.get("content-security-policy"),
          `/assets/${servedWorkerName}`,
        );
        assert.equal(cachedPolicy, policies[0], "Cached Worker must retain the new CSP");
        await context.setOffline(true);
        await page.reload();
        await registerAndControl(page);
        assert.equal(
          await page.title(),
          "SQLite CSP test",
          "Updated shell must still work offline",
        );
      }
      console.log(
        `${engine.name()}: ${warm ? `warm-cache ${mode} CSP upgrade and offline reload` : "cold-cache SQLite persistence"} passed`,
      );
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}
