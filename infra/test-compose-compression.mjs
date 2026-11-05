// Exercise the deployed nginx configuration against real Compose assets.
// node infra/test-compose-compression.mjs http://127.0.0.1:3101 <asset-directory>
import assert from "node:assert/strict";
import { readdir } from "node:fs/promises";
import { request } from "node:http";
import { request as secureRequest } from "node:https";
import { join, relative, sep } from "node:path";
import { gunzipSync } from "node:zlib";

const base = new URL(process.argv[2] ?? "http://127.0.0.1:3101");
const directory = process.argv[3] ?? "frontend/public/compose";
async function files(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  return (await Promise.all(entries.map(entry => entry.isDirectory() ? files(join(dir, entry.name)) : [join(dir, entry.name)]))).flat();
}
function get(url, encoding) {
  return new Promise((resolve, reject) => {
    const send = url.protocol === "https:" ? secureRequest : request;
    send(url, { headers: { "Accept-Encoding": encoding } }, response => {
      const chunks = [];
      response.on("data", chunk => chunks.push(chunk));
      response.on("end", () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks) }));
      response.on("error", reject);
    }).on("error", reject).end();
  });
}
const results = [];
for (const file of (await files(directory)).filter(file => /\.(wasm|js|ttf|otf)$/.test(file))) {
  const path = relative(directory, file).split(sep).map(encodeURIComponent).join("/");
  const url = new URL(`/compose/${path}`, base);
  const identity = await get(url, "identity");
  assert.equal(identity.status, 200, path);
  if (identity.body.length < 1024) continue;
  const compressed = await get(url, "gzip");
  assert.equal(compressed.status, 200, path);
  assert.equal(compressed.headers["content-encoding"], "gzip", path);
  assert.match(compressed.headers.vary ?? "", /Accept-Encoding/i, path);
  assert.ok(gunzipSync(compressed.body).equals(identity.body), `gzip changed ${path}`);
  assert.equal(compressed.headers["cache-control"], identity.headers["cache-control"], path);
  assert.equal(compressed.headers["content-security-policy"], identity.headers["content-security-policy"], path);
  results.push({ path, identityBytes: identity.body.length, gzipBytes: compressed.body.length });
}
assert.ok(results.some(row => row.path.endsWith(".wasm")), "Check at least one Wasm module");
assert.ok(results.some(row => /\.(ttf|otf)$/.test(row.path)), "Check real font resources");
console.log(JSON.stringify({ results, identityBytes: results.reduce((sum, row) => sum + row.identityBytes, 0), gzipBytes: results.reduce((sum, row) => sum + row.gzipBytes, 0) }, null, 2));
