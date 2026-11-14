import * as SQLite from "@journeyapps/wa-sqlite";
import { MemoryVFS } from "@journeyapps/wa-sqlite/src/examples/MemoryVFS.js";

import type { InboundSessionKey } from "../store/backend";
import { CRYPTO_SCHEMA_VERSION } from "../store/schema";
import type { SnapshotBackend } from "./pushDecrypt";

/** 暗号ストアのファイルの写し（本体と、残っていればロールバックの journal）。 */
export interface CryptoDatabaseFiles {
  database: ArrayBuffer;
  journal: ArrayBuffer | null;
}

type Row = Record<string, SQLiteCompatibleType | null>;

let vfsSequence = 0;

/** wa-sqlite 1.4.1 の `MemoryVFS`（パッケージの型は古い API のままなので、使う分だけ型を付ける）。 */
export interface MemoryVfs extends SQLiteVFS {
  mapNameToFile: Map<string, { pathname: string; flags: number; size: number; data: ArrayBuffer }>;
  close(): void;
}

export function createMemoryVfs(name: string, module: unknown): Promise<MemoryVfs> {
  return (
    MemoryVFS as unknown as { create(name: string, module: unknown): Promise<MemoryVfs> }
  ).create(name, module);
}

/**
 * 暗号ストアの写しを、メモリーの SQLite で読む（Push を復号する Service Worker が使う）。
 *
 * Service Worker では、暗号ストアの Worker が使う `OPFSCoopSyncVFS` を使えない（OPFS の同期の
 * ハンドルは専用の Worker にしかない）。そこで、ロックを取ってファイルを読み、メモリーの VFS に
 * 置いて開く。書き込みはメモリーの写しにだけ入り、OPFS には戻らない。journal が残っていれば
 * （書き込みの途中で終了した）、SQLite が写しの上でロールバックする。
 *
 * `module` は wa-sqlite の Emscripten のモジュール（同期版の `wa-sqlite.mjs`）。
 */
export async function openSnapshotBackend(
  module: unknown,
  files: CryptoDatabaseFiles,
): Promise<SnapshotBackend> {
  const sqlite3 = SQLite.Factory(module);
  const vfsName = `zoo-crypto-push-${++vfsSequence}`;
  const vfs = await createMemoryVfs(vfsName, module);
  const place = (pathname: string, data: ArrayBuffer, flags: number) =>
    vfs.mapNameToFile.set(pathname, { pathname, flags, size: data.byteLength, data });
  place("/crypto.sqlite", files.database, SQLite.SQLITE_OPEN_MAIN_DB);
  if (files.journal)
    place("/crypto.sqlite-journal", files.journal, SQLite.SQLITE_OPEN_MAIN_JOURNAL);
  sqlite3.vfs_register(vfs, false);
  const db = await sqlite3.open_v2("/crypto.sqlite", SQLite.SQLITE_OPEN_READWRITE, vfsName);

  async function query(sql: string, params: SQLiteCompatibleType[] = []): Promise<Row[]> {
    const rows: Row[] = [];
    for await (const stmt of sqlite3.statements(db, sql)) {
      if (params.length > 0) sqlite3.bind_collection(stmt, params);
      const columns = sqlite3.column_names(stmt);
      while ((await sqlite3.step(stmt)) === SQLite.SQLITE_ROW) {
        const values = sqlite3.row(stmt);
        rows.push(Object.fromEntries(columns.map((name, index) => [name, values[index] ?? null])));
      }
    }
    return rows;
  }

  const close = async () => {
    await sqlite3.close(db).catch(() => undefined);
    vfs.close();
    vfs.mapNameToFile.clear();
  };

  try {
    const version = Number((await query("PRAGMA user_version"))[0]?.user_version ?? 0);
    // 新しいアプリが表の形を変えたら読まない（復号せずに出す）。
    if (version !== CRYPTO_SCHEMA_VERSION) throw new Error(`暗号ストアの版 ${version} は読めない`);
  } catch (error) {
    await close();
    throw error;
  }

  return {
    async read() {
      const meta = await query("SELECT key, value FROM meta");
      const account = await query("SELECT pickle FROM account WHERE id = 1");
      return {
        meta: Object.fromEntries(meta.map((row) => [String(row.key), String(row.value)])),
        account: account[0] ? String(account[0].pickle) : null,
        // Push の復号には使わないので読まない。
        olmSessions: [],
        trustStore: null,
        processedToDevice: [],
      };
    },
    async readInboundSession(key: InboundSessionKey) {
      const rows = await query("SELECT pickle FROM inbound_sessions WHERE key = ?", [key]);
      return rows[0] ? String(rows[0].pickle) : null;
    },
    close,
  };
}

/**
 * OPFS から暗号ストアのファイルを読む。なければ `null`。
 *
 * 暗号ストアのロック（`cryptoLockName`）を持ったまま呼ぶ。ロックがあれば、暗号ストアの Worker
 * はファイルを開いていないので、読んでいるあいだに書き換わらない。
 */
export async function readCryptoDatabaseFiles(name: string): Promise<CryptoDatabaseFiles | null> {
  const root = await navigator.storage.getDirectory();
  const read = async (fileName: string) => {
    try {
      const handle = await root.getFileHandle(fileName);
      return await (await handle.getFile()).arrayBuffer();
    } catch (error) {
      if ((error as { name?: string }).name === "NotFoundError") return null;
      throw error;
    }
  };
  const database = await read(name);
  if (!database || database.byteLength === 0) return null;
  return { database, journal: await read(`${name}-journal`) };
}
