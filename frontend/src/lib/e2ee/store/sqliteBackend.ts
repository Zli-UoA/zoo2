import * as SQLite from "@journeyapps/wa-sqlite";
import SQLiteESMFactory from "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs";
import { OPFSCoopSyncVFS } from "@journeyapps/wa-sqlite/src/examples/OPFSCoopSyncVFS.js";

import type {
  CryptoBackend,
  CryptoStoreSnapshot,
  CryptoStoreWrite,
  DeviceReplacement,
  InboundSessionKey,
} from "./backend";
import { CRYPTO_SCHEMA, CRYPTO_SCHEMA_VERSION } from "./schema";

type Row = Record<string, SQLiteCompatibleType | null>;

let sqliteReady: Promise<SQLiteAPI> | null = null;
/**
 * `OPFSCoopSyncVFS` が持つ、ファイルのハンドルのキャッシュ（wa-sqlite 1.4.1 の公開フィールド）。
 * 接続を閉じてもハンドルを持ち続けるので、ファイルを消したら外す。外さないと、同じ Worker で
 * 開き直したときに、消したファイルのハンドルを使って失敗する。
 */
let vfsCaches: { persistentFiles: Map<string, unknown>; accessiblePaths: Set<string> } | null =
  null;

/** wa-sqlite と OPFS の VFS を、Worker で 1 回だけ用意する。 */
function loadSqlite(): Promise<SQLiteAPI> {
  sqliteReady ??= (async () => {
    const module = await SQLiteESMFactory();
    const sqlite3 = SQLite.Factory(module);
    // オフライン DB の Worker（@tanstack/browser-db-sqlite-persistence）と同じ VFS。
    // 別の Worker の接続とは、VFS が Web Locks でファイルを譲り合う。
    const vfs = await OPFSCoopSyncVFS.create("zoo-crypto-opfs", module);
    vfsCaches = vfs as unknown as typeof vfsCaches;
    sqlite3.vfs_register(vfs, true);
    return sqlite3;
  })().catch((error: unknown) => {
    // 一時的な失敗なら、次の open で初期化し直す。
    sqliteReady = null;
    throw error;
  });
  return sqliteReady;
}

/**
 * OPFS の SQLite に置く暗号ストア（Worker の中だけで使う）。
 *
 * `databaseName` は `cryptoDatabaseName`（`lib/e2ee/identity.ts`）の値。
 */
export async function openSqliteCryptoBackend(databaseName: string): Promise<CryptoBackend> {
  const sqlite3 = await loadSqlite();
  const db = await sqlite3.open_v2(databaseName);

  async function query(sql: string, params: unknown[] = []): Promise<Row[]> {
    const rows: Row[] = [];
    for await (const stmt of sqlite3.statements(db, sql)) {
      if (params.length > 0) {
        sqlite3.bind_collection(stmt, params as SQLiteCompatibleType[]);
      }
      const columns = sqlite3.column_names(stmt);
      while ((await sqlite3.step(stmt)) === SQLite.SQLITE_ROW) {
        const values = sqlite3.row(stmt);
        rows.push(Object.fromEntries(columns.map((name, index) => [name, values[index] ?? null])));
      }
    }
    return rows;
  }

  // 同じ接続への呼び出しを直列にする（BEGIN と COMMIT の間に別の文が入らないように）。
  let queue: Promise<unknown> = Promise.resolve();
  function enqueue<T>(task: () => Promise<T>): Promise<T> {
    const next = queue.then(task, task);
    queue = next.catch(() => undefined);
    return next;
  }

  const version = Number((await query("PRAGMA user_version"))[0]?.user_version ?? 0);
  if (version > CRYPTO_SCHEMA_VERSION) {
    await sqlite3.close(db);
    throw new Error(`暗号ストアの版 ${version} は、このアプリより新しい`);
  }
  await sqlite3.exec(db, CRYPTO_SCHEMA);

  let closed = false;
  const close = () =>
    enqueue(async () => {
      if (closed) return;
      closed = true;
      await sqlite3.close(db);
    });

  return {
    read: () =>
      enqueue(async (): Promise<CryptoStoreSnapshot> => {
        const meta = await query("SELECT key, value FROM meta");
        const account = await query("SELECT pickle FROM account WHERE id = 1");
        const olm = await query("SELECT their_identity_key, session_id, pickle FROM olm_sessions");
        const trust = await query("SELECT pickle FROM trust_store WHERE id = 1");
        const processed = await query("SELECT id FROM processed_to_device");
        return {
          meta: Object.fromEntries(meta.map((row) => [String(row.key), String(row.value)])),
          account: account[0] ? String(account[0].pickle) : null,
          olmSessions: olm.map((row) => ({
            theirIdentityKey: String(row.their_identity_key),
            sessionId: String(row.session_id),
            pickle: String(row.pickle),
          })),
          trustStore: trust[0] ? String(trust[0].pickle) : null,
          processedToDevice: processed.map((row) => String(row.id)),
        };
      }),

    readInboundSession: (key: InboundSessionKey) =>
      enqueue(async () => {
        const rows = await query("SELECT pickle FROM inbound_sessions WHERE key = ?", [key]);
        return rows[0] ? String(rows[0].pickle) : null;
      }),

    write: (changes: CryptoStoreWrite) =>
      enqueue(async () => {
        await sqlite3.exec(db, "BEGIN IMMEDIATE");
        try {
          for (const [key, value] of Object.entries(changes.meta ?? {})) {
            if (value === null) await query("DELETE FROM meta WHERE key = ?", [key]);
            else
              await query(
                "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, value],
              );
          }
          if (changes.account !== undefined) {
            await query(
              "INSERT INTO account (id, pickle) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET pickle = excluded.pickle",
              [changes.account],
            );
          }
          for (const session of changes.olmSessions ?? []) {
            await query(
              `INSERT INTO olm_sessions (their_identity_key, session_id, pickle) VALUES (?, ?, ?)
               ON CONFLICT (their_identity_key, session_id) DO UPDATE SET pickle = excluded.pickle`,
              [session.theirIdentityKey, session.sessionId, session.pickle],
            );
          }
          for (const { key, pickle, fromBackup } of changes.inboundSessions ?? []) {
            // 鍵バックアップから取り込んだだけなら、同じ版には上げ直さない。新しい行は送信済みにし、
            // すでにある行は送信待ちかどうかを変えない（送っていない Olm 由来のセッションは送る）。
            await query(
              `INSERT INTO inbound_sessions (key, pickle, backed_up) VALUES (?, ?, ?)
               ON CONFLICT (key) DO UPDATE SET pickle = excluded.pickle,
                 backed_up = CASE WHEN excluded.backed_up = 1 THEN inbound_sessions.backed_up ELSE 0 END`,
              [key, pickle, fromBackup ? 1 : 0],
            );
          }
          if (changes.trustStore !== undefined) {
            await query(
              "INSERT INTO trust_store (id, pickle) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET pickle = excluded.pickle",
              [changes.trustStore],
            );
          }
          for (const id of changes.addProcessedToDevice ?? []) {
            await query("INSERT OR IGNORE INTO processed_to_device (id) VALUES (?)", [id]);
          }
          for (const id of changes.removeProcessedToDevice ?? []) {
            await query("DELETE FROM processed_to_device WHERE id = ?", [id]);
          }
          await sqlite3.exec(db, "COMMIT");
        } catch (error) {
          await sqlite3.exec(db, "ROLLBACK").catch(() => undefined);
          throw error;
        }
      }),

    pendingBackupCount: () =>
      enqueue(async () => {
        const rows = await query("SELECT count(*) AS n FROM inbound_sessions WHERE backed_up = 0");
        return Number(rows[0]?.n ?? 0);
      }),

    pendingBackupSessions: (limit: number) =>
      enqueue(async () => {
        const rows = await query(
          "SELECT key, pickle FROM inbound_sessions WHERE backed_up = 0 ORDER BY key LIMIT ?",
          [limit],
        );
        return rows.map((row) => ({ key: String(row.key), pickle: String(row.pickle) }));
      }),

    markBackedUp: (sessions: { key: InboundSessionKey; pickle: string }[]) =>
      enqueue(async () => {
        await sqlite3.exec(db, "BEGIN IMMEDIATE");
        try {
          for (const { key, pickle } of sessions) {
            // 送ったあとに書き換わったセッションは、送信待ちのまま残す。
            await query("UPDATE inbound_sessions SET backed_up = 1 WHERE key = ? AND pickle = ?", [
              key,
              pickle,
            ]);
          }
          await sqlite3.exec(db, "COMMIT");
        } catch (error) {
          await sqlite3.exec(db, "ROLLBACK").catch(() => undefined);
          throw error;
        }
      }),

    resetBackedUp: () =>
      enqueue(async () => {
        await query("UPDATE inbound_sessions SET backed_up = 0");
      }),

    readAllInboundSessions: () =>
      enqueue(async () => {
        const rows = await query("SELECT key, pickle, backed_up FROM inbound_sessions");
        return rows.map((row) => ({
          key: String(row.key),
          pickle: String(row.pickle),
          backedUp: Number(row.backed_up) === 1,
        }));
      }),

    replaceDevice: (replacement: DeviceReplacement) =>
      enqueue(async () => {
        await sqlite3.exec(db, "BEGIN IMMEDIATE");
        try {
          await query("DELETE FROM meta");
          for (const [key, value] of Object.entries(replacement.meta)) {
            await query("INSERT INTO meta (key, value) VALUES (?, ?)", [key, value]);
          }
          await query(
            "INSERT INTO account (id, pickle) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET pickle = excluded.pickle",
            [replacement.account],
          );
          await query(
            "INSERT INTO trust_store (id, pickle) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET pickle = excluded.pickle",
            [replacement.trustStore],
          );
          await query("DELETE FROM olm_sessions");
          await query("DELETE FROM processed_to_device");
          // 封をし直した受信用セッションに置き換える（送ったかどうかは変えない）。開けなかった行は消す。
          for (const { key, pickle } of replacement.inboundSessions) {
            await query("UPDATE inbound_sessions SET pickle = ? WHERE key = ?", [pickle, key]);
          }
          for (const key of replacement.droppedInboundSessions) {
            await query("DELETE FROM inbound_sessions WHERE key = ?", [key]);
          }
          await sqlite3.exec(db, "COMMIT");
        } catch (error) {
          await sqlite3.exec(db, "ROLLBACK").catch(() => undefined);
          throw error;
        }
      }),

    close,

    async destroy() {
      await close();
      await removeCryptoDatabaseFiles(databaseName);
    },
  };
}

/** 暗号ストアのファイルを消す。開いている接続は先に閉じておく。 */
export async function removeCryptoDatabaseFiles(databaseName: string): Promise<void> {
  const root = await navigator.storage.getDirectory();
  for (const suffix of ["", "-journal", "-wal"]) {
    // VFS はパスを `new URL(name, "file://").pathname` で持つ。
    const path = `/${databaseName}${suffix}`;
    vfsCaches?.persistentFiles.delete(path);
    vfsCaches?.accessiblePaths.delete(path);
    try {
      await root.removeEntry(`${databaseName}${suffix}`);
    } catch (error) {
      if ((error as { name?: string }).name !== "NotFoundError") throw error;
    }
  }
}
