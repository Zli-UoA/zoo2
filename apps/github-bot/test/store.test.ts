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

test("a successful current Check refresh covers duplicates but preserves new arrivals and history", () => {
  const store = new Store(":memory:", Buffer.alloc(32, 1));
  const base = { repository: { full_name: "Acme/Project" }, installation: { id: 10 } };
  const check = (sha = "a".repeat(40), prs: number[] = []) => ({
    ...base, check_run: { head_sha: sha, pull_requests: prs.map((number) => ({ number })) },
  });
  store.enqueue("old", "github", "check_run", check());
  store.enqueue("comment", "github", "issue_comment", base);
  store.enqueue("latest", "github", "check_run", check());
  store.enqueue("other-sha", "github", "check_run", check("b".repeat(40)));
  store.enqueue("other-pr", "github", "check_run", check("a".repeat(40), [7]));
  store.enqueue("other-installation", "github", "check_run", { ...check(), installation: { id: 11 } });
  store.enqueue("other-repo", "github", "check_run", { ...check(), repository: { full_name: "acme/other" } });
  store.db.exec("UPDATE jobs SET created=rowid");
  const latest = store.next()!;
  assert.equal(latest.id, "latest");
  store.enqueue("arrived-during-request", "github", "check_run", check());
  store.done(latest);
  assert.equal(store.db.prepare("SELECT status FROM jobs WHERE id='old'").get()!.status, "done");
  for (const id of ["comment", "other-sha", "other-pr", "other-installation", "other-repo", "arrived-during-request"])
    assert.equal(store.db.prepare("SELECT status FROM jobs WHERE id=?").get(id)!.status, "queued");
  assert.equal(store.enqueue("old", "github", "check_run", check()), false);
  assert.equal(store.next()!.id, "comment");
  store.close();
});

test("failed Check refreshes do not acknowledge older queued updates", () => {
  const store = new Store(":memory:", Buffer.alloc(32, 1));
  const base = { repository: { full_name: "acme/project" }, installation: { id: 10 } };
  const sha = "a".repeat(40);
  store.enqueue("status", "github", "status", { ...base, sha });
  store.enqueue("suite", "github", "check_suite", { ...base, check_suite: { head_sha: sha } });
  const job = store.next()!;
  assert.equal(job.id, "suite");
  store.fail(job, "unavailable", true);
  assert.equal(store.db.prepare("SELECT count(*) n FROM jobs WHERE status='queued'").get()!.n, 2);
  const retry = store.next()!;
  assert.equal(retry.id, "status", "A newer retry's delay must not block ready work");
  store.done(retry);
  assert.equal(store.db.prepare("SELECT status FROM jobs WHERE id='suite'").get()!.status, "queued");
  store.close();
});

test("an existing SQLite backlog is migrated and coalesced after worker acquisition", async (t) => {
  const { DatabaseSync } = await import("node:sqlite");
  const dir = mkdtempSync(join(tmpdir(), "zoo-github-old-queue-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const path = join(dir, "bot.sqlite");
  const old = new DatabaseSync(path);
  old.exec("CREATE TABLE jobs(id TEXT PRIMARY KEY,source TEXT,event TEXT,payload TEXT,fingerprint TEXT,status TEXT DEFAULT 'queued',attempts INTEGER DEFAULT 0,available INTEGER,created INTEGER,error TEXT)");
  const payload = JSON.stringify({ repository: { full_name: "acme/project" }, installation: { id: 10 }, sha: "a".repeat(40) });
  for (const id of ["old", "new"])
    old.prepare("INSERT INTO jobs(id,source,event,payload,available,created) VALUES(?,'github','status',?,0,0)").run(id, payload);
  old.close();
  const store = new Store(path, Buffer.alloc(32, 1));
  const owner = store.acquireWorker();
  const job = store.next()!;
  assert.equal(job.id, "new");
  store.done(job);
  assert.equal(store.db.prepare("SELECT count(*) n FROM jobs WHERE status='done'").get()!.n, 2);
  store.releaseWorker(owner);
  store.close();
});
