import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import http from "node:http";
import { chromium, firefox } from "@playwright/test";

// Exercise the actual bundled SQLite Worker under the deployed CSP, without an API server.
const assets = new URL("../dist/client/assets/", import.meta.url);
const workerName = (await readdir(assets)).find((name) => /^opfs-worker-.*\.js$/.test(name));
assert.ok(workerName, "Run pnpm build before this test");
const workerSource = await readFile(new URL(workerName, assets));
const nginx = await readFile(new URL("../../infra/frontend-nginx.conf", import.meta.url), "utf8");
const policies = [...nginx.matchAll(/add_header Content-Security-Policy "([^"]+)" always;/g)].map(
  (match) => match[1].replaceAll("${CSP_SCRIPT_SRC}", "").replaceAll("${CSP_CONNECT_SRC}", ""),
);
assert.equal(policies.length, 2);
assert.equal(policies[0], policies[1], "Document and Worker policies must agree");
const server = http.createServer((request, response) => {
  response.setHeader("Content-Security-Policy", policies[0]);
  response.setHeader("Content-Type", request.url === "/" ? "text/html" : "text/javascript");
  response.end(
    request.url === "/" ? "<!doctype html><title>SQLite CSP test</title>" : workerSource,
  );
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
try {
  for (const engine of [firefox, chromium]) {
    const browser = await engine.launch();
    try {
      const page = await browser.newPage();
      await page.goto(origin);
      const rows = await page.evaluate(async (workerName) => {
        async function connect() {
          const worker = new Worker(`/${workerName}`);
          let sequence = 0;
          function request(payload) {
            const requestId = String(++sequence);
            return new Promise((resolve, reject) => {
              const timeout = setTimeout(() => {
                worker.terminate();
                reject(new Error("SQLite Worker timed out"));
              }, 8000);
              worker.onmessage = ({ data }) => {
                if (data.requestId !== requestId) return;
                clearTimeout(timeout);
                if (data.ok) resolve(data.rows);
                else reject(new Error(data.error));
              };
              worker.onerror = (event) => {
                clearTimeout(timeout);
                reject(new Error(event.message));
              };
              worker.postMessage({ params: [], ...payload, requestId });
            });
          }
          await request({ type: "init", databaseName: "csp-test.sqlite", vfsName: "opfs" });
          return { worker, request };
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
      }, workerName);
      assert.deepEqual(rows, [{ content: "保存確認" }]);
      console.log(`${engine.name()}: SQLite persists across Worker restart under production CSP`);
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}
