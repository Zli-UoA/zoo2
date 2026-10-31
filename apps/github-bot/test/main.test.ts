import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pair } from "./helpers.ts";
import { loadConfig } from "../src/config.ts";

test("the production entrypoint starts, preserves credentials, exposes health, and shuts down cleanly", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-main-"));
  const zoo = createServer((req, res) => {
    if (req.url === "/api/apps/v1/auth") {
      assert.equal(req.headers.authorization, "Bearer zoo-access");
      res
        .writeHead(200, { "content-type": "application/json" })
        .end(JSON.stringify({ app_id: "app", bot_user_id: "bot" }));
    } else res.writeHead(404).end();
  });
  await new Promise<void>((resolve) => zoo.listen(0, "127.0.0.1", resolve));
  const zooPort = (zoo.address() as { port: number }).port;
  const reserve = createServer();
  await new Promise<void>((resolve) => reserve.listen(0, "127.0.0.1", resolve));
  const port = (reserve.address() as { port: number }).port;
  await new Promise<void>((resolve) => reserve.close(() => resolve()));
  writeFileSync(
    join(dir, "config.json"),
    JSON.stringify({
      repositories: [{ repository: "acme/project", installationId: 10, channels: ["channel"] }],
    }),
  );
  writeFileSync(join(dir, "key.pem"), pair.privateKey.export({ type: "pkcs8", format: "pem" }));
  const env = {
    ...process.env,
    BOT_CONFIG: join(dir, "config.json"),
    BOT_DATABASE: join(dir, "data/bot.sqlite"),
    BOT_ENCRYPTION_KEY: Buffer.alloc(32, 9).toString("base64"),
    ZOO_URL: `http://127.0.0.1:${zooPort}`,
    ZOO_ACCESS_TOKEN: "zoo-access",
    ZOO_REFRESH_TOKEN: "zoo-refresh",
    ZOO_SIGNING_SECRET: "zoo-secret",
    GITHUB_APP_ID: "42",
    GITHUB_PRIVATE_KEY_FILE: join(dir, "key.pem"),
    GITHUB_WEBHOOK_SECRET: "github-secret",
    HOST: "127.0.0.1",
    PORT: String(port),
  };
  const child = spawn(
    process.execPath,
    ["--experimental-strip-types", new URL("../src/main.ts", import.meta.url).pathname],
    { env, stdio: ["ignore", "pipe", "pipe"] },
  );
  let output = "";
  child.stdout.on("data", (b) => (output += b));
  child.stderr.on("data", (b) => (output += b));
  t.after(async () => {
    if (child.exitCode === null) child.kill("SIGTERM");
    zoo.closeAllConnections();
    await new Promise<void>((resolve) => zoo.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  });
  const deadline = Date.now() + 10000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/health`);
      if (response.ok) {
        ready = true;
        break;
      }
    } catch {}
    if (child.exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.equal(ready, true, output);
  const ended = once(child, "exit");
  child.kill("SIGTERM");
  const [code] = await ended;
  assert.equal(code, 0, output);
  assert.ok(!output.includes("zoo-access"));
  assert.ok(!output.includes("zoo-refresh"));
  assert.equal(loadConfig(env).rules[0].repository, "acme/project");
  assert.throws(() => loadConfig({ ...env, ZOO_URL: "https://user:pass@zoo.example" }), /origin/);
  assert.throws(() => loadConfig({ ...env, BOT_ENCRYPTION_KEY: "short" }), /32 random/);
  writeFileSync(
    join(dir, "config.json"),
    JSON.stringify({
      repositories: [
        { repository: "acme/*", installationId: 10, channels: ["a"] },
        { repository: "acme/project", installationId: 11, channels: ["b"] },
      ],
    }),
  );
  assert.throws(() => loadConfig(env), /same installation/);
});

test("the supplied GitHub App has read-only permissions and Zoo subscribes to no chat mutation events", async () => {
  const { readFileSync } = await import("node:fs");
  const github = JSON.parse(
    readFileSync(new URL("../github-app-manifest.example.json", import.meta.url), "utf8"),
  );
  assert.ok(Object.values(github.default_permissions).every((v) => v === "read"));
  assert.equal(github.request_oauth_on_install, false);
  const zoo = JSON.parse(
    readFileSync(new URL("../manifest.example.json", import.meta.url), "utf8"),
  );
  assert.ok(
    !zoo.events.some((e: string) =>
      ["message.created", "message.updated", "message.deleted", "action.invoked"].includes(e),
    ),
  );
  assert.ok(!zoo.scopes.includes("messages:read"));
});
