import { test } from "node:test";
import assert from "node:assert/strict";
import { createHmac, randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { server } from "../src/server.ts";
import { Store } from "../src/store.ts";
import { GitHub } from "../src/github.ts";
import { GitHubBot } from "../src/bot.ts";
import { fixture, job, command } from "./helpers.ts";
async function serve(f: ReturnType<typeof fixture>) {
  const http = server(f.config, f.store);
  await new Promise<void>((resolve) => http.listen(0, "127.0.0.1", resolve));
  const port = (http.address() as { port: number }).port;
  return {
    url: `http://127.0.0.1:${port}`,
    close: async () => {
      http.closeAllConnections();
      await new Promise<void>((resolve) => http.close(() => resolve()));
    },
  };
}
function githubHeaders(secret: string, raw: string, id = randomUUID()) {
  return {
    "content-type": "application/json",
    "x-github-event": "issues",
    "x-github-delivery": id,
    "x-hub-signature-256": `sha256=${createHmac("sha256", secret).update(raw).digest("hex")}`,
  };
}
function zooHeaders(secret: string, raw: string, age = 0) {
  const time = String(Math.floor(Date.now() / 1000) - age);
  return {
    "content-type": "application/json",
    "x-zoo-timestamp": time,
    "x-zoo-signature": `v1=${createHmac("sha256", secret).update(`v1:${time}:${raw}`).digest("hex")}`,
  };
}

test("GitHub authentication uses the exact body, checks installation policy, and persists before ACK", async (t) => {
  const f = fixture(),
    http = await serve(f);
  t.after(async () => {
    await http.close();
    f.store.close();
  });
  const raw = JSON.stringify(job("issues", { action: "opened", issue: f.remote.issue }).payload),
    headers = githubHeaders(f.config.githubSecret, raw);
  assert.equal(
    (await fetch(`${http.url}/github/webhook`, { method: "POST", headers, body: raw + " " }))
      .status,
    401,
  );
  assert.equal(
    (await fetch(`${http.url}/github/webhook`, { method: "POST", headers, body: raw })).status,
    202,
  );
  assert.equal(f.zoo.messages.size, 0);
  assert.equal(f.store.db.prepare("SELECT status FROM jobs").get()!.status, "queued");
  await fetch(`${http.url}/github/webhook`, { method: "POST", headers, body: raw });
  assert.equal(f.store.db.prepare("SELECT count(*) AS n FROM jobs").get()!.n, 1);
  const wrong = JSON.stringify(
    job("issues", { installation: { id: 999 }, action: "opened", issue: f.remote.issue }).payload,
  );
  await fetch(`${http.url}/github/webhook`, {
    method: "POST",
    headers: githubHeaders(f.config.githubSecret, wrong),
    body: wrong,
  });
  assert.equal(f.store.db.prepare("SELECT count(*) AS n FROM jobs").get()!.n, 1);
  await f.bot.tick();
  assert.equal(f.zoo.messages.size, 1);
  assert.equal(f.store.db.prepare("SELECT status FROM jobs").get()!.status, "done");
});

test("Zoo signature expiry, URL verification, and ignored chat replies cannot trigger GitHub writes", async (t) => {
  const f = fixture(),
    http = await serve(f);
  t.after(async () => {
    await http.close();
    f.store.close();
  });
  const challenge = JSON.stringify({ type: "url_verification", challenge: "verify" });
  const verified = await fetch(`${http.url}/zoo/events`, {
    method: "POST",
    headers: zooHeaders(f.config.zooSecret, challenge),
    body: challenge,
  });
  assert.deepEqual(await verified.json(), { challenge: "verify" });
  const raw = JSON.stringify({
    delivery_id: randomUUID(),
    event: command("subscribe list features"),
  });
  assert.equal(
    (
      await fetch(`${http.url}/zoo/events`, {
        method: "POST",
        headers: zooHeaders(f.config.zooSecret, raw, 600),
        body: raw,
      })
    ).status,
    401,
  );
  const chat = JSON.stringify({
    event: {
      ...command(""),
      type: "message.created",
      data: { content: "merge acme/project#8", parent_id: "thread" },
    },
  });
  assert.equal(
    (
      await fetch(`${http.url}/zoo/events`, {
        method: "POST",
        headers: zooHeaders(f.config.zooSecret, chat),
        body: chat,
      })
    ).status,
    202,
  );
  assert.equal(f.store.db.prepare("SELECT count(*) AS n FROM jobs").get()!.n, 0);
  assert.equal(f.remote.readRequests.length, 0);
  assert.equal(
    (
      await fetch(`${http.url}/zoo/events`, {
        method: "POST",
        headers: zooHeaders(f.config.zooSecret, raw),
        body: raw,
      })
    ).status,
    202,
  );
  await f.bot.tick();
  assert.equal(f.remote.readRequests.length, 0);
});

test("a persisted webhook survives process restart and re-delivery without a second post", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-server-")),
    path = join(dir, "queue.sqlite"),
    f = fixture({ database: path }),
    http = await serve(f);
  const raw = JSON.stringify(job("issues", { action: "opened", issue: f.remote.issue }).payload),
    headers = githubHeaders(f.config.githubSecret, raw);
  await fetch(`${http.url}/github/webhook`, { method: "POST", headers, body: raw });
  await http.close();
  f.store.close();
  const store = new Store(path, f.config.encryptionKey),
    owner = store.acquireWorker(),
    github = new GitHub(f.config, store, f.fetcher),
    bot = new GitHubBot(f.config, store, github, f.zoo);
  t.after(() => {
    store.releaseWorker(owner);
    store.close();
    rmSync(dir, { recursive: true, force: true });
  });
  await bot.tick();
  assert.equal(f.zoo.messages.size, 1);
  assert.equal(
    store.enqueue(`github:${headers["x-github-delivery"]}`, "github", "issues", JSON.parse(raw)),
    false,
  );
  await bot.tick();
  assert.equal(f.zoo.messages.size, 1);
});

test("malformed signed payloads are rejected without persisting a job", async (t) => {
  const f = fixture(),
    http = await serve(f);
  t.after(async () => {
    await http.close();
    f.store.close();
  });
  const invalid = "{";
  assert.equal(
    (
      await fetch(`${http.url}/github/webhook`, {
        method: "POST",
        headers: githubHeaders(f.config.githubSecret, invalid),
        body: invalid,
      })
    ).status,
    400,
  );
  const empty = JSON.stringify({ event: { id: randomUUID(), type: "command.invoked", data: [] } });
  assert.equal(
    (
      await fetch(`${http.url}/zoo/events`, {
        method: "POST",
        headers: zooHeaders(f.config.zooSecret, empty),
        body: empty,
      })
    ).status,
    400,
  );
  assert.equal(f.store.db.prepare("SELECT count(*) AS n FROM jobs").get()!.n, 0);
});

test("recovery after publishing a parent but before ACK does not add an extra opening reply", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-recovery-")),
    path = join(dir, "queue.sqlite"),
    f = fixture({ database: path });
  const event = job("issues", { action: "opened", issue: f.remote.issue });
  f.store.enqueue(event.id, "github", event.event, event.payload);
  const unfinished = f.store.next()!;
  await f.bot.githubEvent(unfinished);
  assert.equal(f.zoo.messages.size, 1);
  f.store.close();
  const store = new Store(path, f.config.encryptionKey),
    owner = store.acquireWorker(),
    github = new GitHub(f.config, store, f.fetcher),
    bot = new GitHubBot(f.config, store, github, f.zoo);
  t.after(() => {
    store.releaseWorker(owner);
    store.close();
    rmSync(dir, { recursive: true, force: true });
  });
  await bot.tick();
  assert.equal(f.zoo.messages.size, 1);
});
