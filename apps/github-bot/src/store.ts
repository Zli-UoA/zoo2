import { DatabaseSync } from "node:sqlite";
import { mkdirSync, chmodSync } from "node:fs";
import { dirname } from "node:path";
import { createCipheriv, createDecipheriv, randomBytes, randomUUID } from "node:crypto";
import { BotError, type Json, type Subscription, type Thread, type Target } from "./types.ts";
export interface Job {
  id: string;
  source: "github" | "zoo";
  event: string;
  payload: Json;
  attempts: number;
  created: number;
}
export class Store {
  readonly db: DatabaseSync;
  private readonly key: Buffer;
  constructor(path: string, key: Buffer) {
    this.key = key;
    if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    this.db = new DatabaseSync(path);
    if (path !== ":memory:") chmodSync(path, 0o600);
    this.db.exec(`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
      CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,source TEXT,event TEXT,payload TEXT,fingerprint TEXT,status TEXT DEFAULT 'queued',attempts INTEGER DEFAULT 0,available INTEGER,created INTEGER,error TEXT);
      CREATE INDEX IF NOT EXISTS jobs_ready ON jobs(status,available,created);
      CREATE TABLE IF NOT EXISTS subscriptions(channel TEXT,repo TEXT,value TEXT,PRIMARY KEY(channel,repo));
      CREATE TABLE IF NOT EXISTS threads(channel TEXT,repo TEXT,kind TEXT,number TEXT,message TEXT UNIQUE,snapshot TEXT,PRIMARY KEY(channel,repo,kind,number));
      CREATE TABLE IF NOT EXISTS worker(id INTEGER PRIMARY KEY CHECK(id=1),owner TEXT,expires INTEGER);`);
  }
  acquireWorker(): string {
    const owner = randomUUID();
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const existing = this.db.prepare("SELECT expires FROM worker WHERE id=1").get();
      if (existing && Number(existing.expires) > Date.now())
        throw new BotError("Another worker is using this database");
      this.db
        .prepare(
          "INSERT INTO worker VALUES(1,?,?) ON CONFLICT(id) DO UPDATE SET owner=excluded.owner,expires=excluded.expires",
        )
        .run(owner, Date.now() + 30000);
      this.db.exec("UPDATE jobs SET status='queued' WHERE status='working'; COMMIT");
      return owner;
    } catch (error) {
      this.db.exec("ROLLBACK");
      throw error;
    }
  }
  heartbeat(owner: string): boolean {
    return Boolean(
      this.db
        .prepare("UPDATE worker SET expires=? WHERE id=1 AND owner=?")
        .run(Date.now() + 30000, owner).changes,
    );
  }
  releaseWorker(owner: string) {
    this.db.prepare("DELETE FROM worker WHERE id=1 AND owner=?").run(owner);
  }
  workerActive(): boolean {
    const r = this.db.prepare("SELECT expires FROM worker WHERE id=1").get();
    return Boolean(r && Number(r.expires) > Date.now());
  }
  get<T>(key: string): T | undefined {
    const row = this.db.prepare("SELECT value FROM kv WHERE key=?").get(key);
    return row ? JSON.parse(String(row.value)) : undefined;
  }
  set(key: string, value: unknown) {
    this.db
      .prepare("INSERT INTO kv VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
      .run(key, JSON.stringify(value));
  }
  remove(key: string) {
    this.db.prepare("DELETE FROM kv WHERE key=?").run(key);
  }
  seal(value: unknown, context = ""): string {
    const nonce = randomBytes(12),
      cipher = createCipheriv("aes-256-gcm", this.key, nonce);
    cipher.setAAD(Buffer.from(context));
    return Buffer.concat([
      nonce,
      cipher.update(JSON.stringify(value), "utf8"),
      cipher.final(),
      cipher.getAuthTag(),
    ]).toString("base64");
  }
  unseal<T>(value: string, context = ""): T {
    const bytes = Buffer.from(value, "base64"),
      cipher = createDecipheriv("aes-256-gcm", this.key, bytes.subarray(0, 12));
    cipher.setAAD(Buffer.from(context));
    cipher.setAuthTag(bytes.subarray(-16));
    return JSON.parse(
      Buffer.concat([cipher.update(bytes.subarray(12, -16)), cipher.final()]).toString("utf8"),
    );
  }
  secret<T>(key: string): T | undefined {
    const value = this.get<string>(`secret:${key}`);
    return value ? this.unseal<T>(value, `secret:${key}`) : undefined;
  }
  saveSecret(key: string, value: unknown) {
    this.set(`secret:${key}`, this.seal(value, `secret:${key}`));
  }
  enqueue(
    id: string,
    source: Job["source"],
    event: string,
    payload: Json,
    fingerprint: string | null = null,
  ): boolean {
    this.db.exec("BEGIN IMMEDIATE");
    try {
      if (this.db.prepare("SELECT 1 FROM jobs WHERE id=?").get(id)) {
        this.db.exec("COMMIT");
        return false;
      }
      const body = JSON.stringify(payload),
        load = this.db
          .prepare(
            "SELECT count(*) AS n,coalesce(sum(length(CAST(payload AS BLOB))),0) AS bytes FROM jobs WHERE status IN('queued','working')",
          )
          .get()!;
      if (
        Number(load.n) >= 10000 ||
        Number(load.bytes) + Buffer.byteLength(body) > 256 * 1024 * 1024
      )
        throw new BotError("queue_full", true);
      this.db
        .prepare(
          "INSERT INTO jobs(id,source,event,payload,fingerprint,available,created) VALUES(?,?,?,?,?,?,?)",
        )
        .run(id, source, event, body, fingerprint, Date.now(), Date.now());
      this.db.exec("COMMIT");
      return true;
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
  next(): Job | undefined {
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const r = this.db
        .prepare(
          "SELECT * FROM jobs WHERE status='queued' AND available<=? ORDER BY created,id LIMIT 1",
        )
        .get(Date.now());
      if (!r) {
        this.db.exec("COMMIT");
        return;
      }
      this.db
        .prepare("UPDATE jobs SET status='working',attempts=attempts+1 WHERE id=?")
        .run(String(r.id));
      this.db.exec("COMMIT");
      return {
        id: String(r.id),
        source: r.source as Job["source"],
        event: String(r.event),
        payload: JSON.parse(String(r.payload)),
        attempts: Number(r.attempts) + 1,
        created: Number(r.created),
      };
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
  done(job: Job) {
    this.db.prepare("UPDATE jobs SET status='done',error=NULL WHERE id=?").run(job.id);
  }
  fail(job: Job, code: string, retryable: boolean, retryAfter = 0) {
    this.db
      .prepare("UPDATE jobs SET status=?,available=?,error=? WHERE id=?")
      .run(
        retryable && job.attempts < 12 ? "queued" : "dead",
        Date.now() +
          Math.max(Math.min(3600000, retryAfter), Math.min(300000, 2 ** job.attempts * 1000)),
        code.slice(0, 160),
        job.id,
      );
  }
  subscription(channel: string, repo: string): Subscription | undefined {
    const r = this.db
      .prepare("SELECT value FROM subscriptions WHERE channel=? AND repo=?")
      .get(channel, repo);
    return r ? JSON.parse(String(r.value)) : undefined;
  }
  subscriptions(channel?: string): Subscription[] {
    return (
      channel
        ? this.db
            .prepare("SELECT value FROM subscriptions WHERE channel=? ORDER BY repo")
            .all(channel)
        : this.db.prepare("SELECT value FROM subscriptions ORDER BY channel,repo").all()
    ).map((r) => JSON.parse(String(r.value)));
  }
  subscribe(value: Subscription) {
    this.db
      .prepare(
        "INSERT INTO subscriptions VALUES(?,?,?) ON CONFLICT(channel,repo) DO UPDATE SET value=excluded.value",
      )
      .run(value.channel, value.repo, JSON.stringify(value));
  }
  unsubscribe(channel: string, repo: string) {
    this.db.prepare("DELETE FROM subscriptions WHERE channel=? AND repo=?").run(channel, repo);
  }
  thread(channel: string, target: Target): Thread | undefined {
    const r = this.db
      .prepare("SELECT * FROM threads WHERE channel=? AND repo=? AND kind=? AND number=?")
      .get(channel, target.repo, target.kind, target.number);
    return r
      ? { ...target, channel, message: String(r.message), snapshot: JSON.parse(String(r.snapshot)) }
      : undefined;
  }
  message(channel: string, message: string): Thread | undefined {
    const r = this.db
      .prepare("SELECT * FROM threads WHERE channel=? AND message=?")
      .get(channel, message);
    return r
      ? {
          channel,
          message,
          repo: String(r.repo),
          kind: r.kind as Thread["kind"],
          number: String(r.number),
          snapshot: JSON.parse(String(r.snapshot)),
        }
      : undefined;
  }
  saveThread(thread: Thread) {
    this.db
      .prepare(
        "INSERT INTO threads VALUES(?,?,?,?,?,?) ON CONFLICT(channel,repo,kind,number) DO UPDATE SET snapshot=excluded.snapshot",
      )
      .run(
        thread.channel,
        thread.repo,
        thread.kind,
        thread.number,
        thread.message,
        JSON.stringify(thread.snapshot),
      );
  }
  cleanup() {
    this.db
      .prepare("DELETE FROM jobs WHERE status='done' AND created<?")
      .run(Date.now() - 30 * 86400000);
  }
  close() {
    this.db.close();
  }
}
