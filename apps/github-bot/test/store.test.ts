import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Store } from "../src/store.ts";

test("delivery IDs deduplicate durably, while identical legitimate payloads may recur", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-store-")),
    path = join(dir, "bot.sqlite"),
    key = Buffer.alloc(32, 9);
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  let store = new Store(path, key);
  assert.equal(
    store.enqueue("github:one", "github", "create", { ref: "feature" }, "same-body"),
    true,
  );
  assert.equal(
    store.enqueue("github:one", "github", "create", { ref: "feature" }, "same-body"),
    false,
  );
  const job = store.next()!;
  store.done(job);
  store.close();
  store = new Store(path, key);
  assert.equal(
    store.enqueue("github:one", "github", "create", { ref: "feature" }, "same-body"),
    false,
  );
  assert.equal(
    store.enqueue("github:two", "github", "create", { ref: "feature" }, "same-body"),
    true,
  );
  assert.equal(store.next()!.id, "github:two");
  assert.equal(statSync(path).mode & 0o777, 0o600);
  store.close();
});

test("one worker owns the database and recovers unfinished jobs after a crash", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-worker-")),
    path = join(dir, "bot.sqlite"),
    key = Buffer.alloc(32, 1);
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const first = new Store(path, key),
    owner = first.acquireWorker();
  first.enqueue("pending", "github", "issues", {});
  assert.equal(first.next()!.attempts, 1);
  const other = new Store(path, key);
  assert.throws(() => other.acquireWorker(), /Another worker/);
  assert.equal(
    first.db.prepare("SELECT status FROM jobs WHERE id='pending'").get()!.status,
    "working",
  );
  assert.equal(first.heartbeat(owner), true);
  assert.equal(other.heartbeat("wrong-owner"), false);
  first.db.prepare("UPDATE worker SET expires=0").run();
  first.close();
  const next = other.acquireWorker();
  assert.equal(other.next()!.attempts, 2);
  other.releaseWorker(next);
  assert.equal(other.workerActive(), false);
  other.close();
});

test("rotated Zoo credentials are encrypted and bound to their secret record", () => {
  const store = new Store(":memory:", Buffer.alloc(32, 7));
  store.saveSecret("zoo", { access_token: "never-in-plaintext", refresh_token: "rotated" });
  const raw = String(store.db.prepare("SELECT value FROM kv WHERE key='secret:zoo'").get()!.value);
  assert.ok(!raw.includes("never-in-plaintext"));
  assert.equal(store.secret<any>("zoo")!.refresh_token, "rotated");
  store.set("secret:other", JSON.parse(raw));
  assert.throws(() => store.secret("other"));
  store.close();
});

test("failed jobs retain diagnostics and are bounded by retries without deleting dead jobs", () => {
  const store = new Store(":memory:", Buffer.alloc(32, 1));
  store.enqueue("failure", "github", "issues", {});
  let job = store.next()!;
  store.fail(job, "GitHub API 503", true);
  assert.equal(store.next(), undefined);
  store.db.prepare("UPDATE jobs SET available=0,attempts=11").run();
  job = store.next()!;
  store.fail(job, "GitHub API 503", true);
  assert.equal(
    store.db.prepare("SELECT status FROM jobs WHERE id='failure'").get()!.status,
    "dead",
  );
  store.db.prepare("UPDATE jobs SET created=0").run();
  store.cleanup();
  assert.equal(store.db.prepare("SELECT count(*) AS n FROM jobs").get()!.n, 1);
  store.close();
});
