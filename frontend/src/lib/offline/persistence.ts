import {
  createBrowserWASQLitePersistence,
  openBrowserWASQLiteOPFSDatabase,
  type BrowserWASQLiteDatabase,
  type PersistedCollectionPersistence,
} from "@tanstack/browser-db-sqlite-persistence";

import {
  clearPendingOfflineDeletion,
  offlineDatabaseName,
  readPendingOfflineDeletion,
} from "./identity";

export class OfflinePersistenceUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "OfflinePersistenceUnavailableError";
  }
}

export interface OfflinePersistenceHandle {
  databaseName: string;
  database: BrowserWASQLiteDatabase;
  persistence: PersistedCollectionPersistence;
}

const handles = new Map<string, Promise<OfflinePersistenceHandle>>();

export function supportsOfflinePersistence(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    typeof navigator.storage?.getDirectory === "function" &&
    typeof Worker !== "undefined"
  );
}

/**
 * ユーザー名前空間ごとに OPFS の SQLite DB を1つ開く。
 *
 * DB はユーザー単位で分離し、同じ Promise を共有して複数のコレクションが
 * 同じ接続を使う。失敗した Promise はキャッシュから外し、再試行できる。
 */
export function openOfflinePersistence(
  namespace: string,
  options: { timeoutMs?: number } = {},
): Promise<OfflinePersistenceHandle> {
  const databaseName = offlineDatabaseName(namespace);
  const existing = handles.get(databaseName);
  if (existing) {
    return existing;
  }
  const opening = (async () => {
    if (!supportsOfflinePersistence()) {
      throw new OfflinePersistenceUnavailableError(
        "OPFS and Web Workers are not available in this browser",
      );
    }
    const database = await openBrowserWASQLiteOPFSDatabase({
      databaseName,
      timeoutMs: options.timeoutMs ?? 8_000,
    });
    const persistence = createBrowserWASQLitePersistence({
      database,
      schemaMismatchPolicy: "sync-present-reset",
    });
    return { databaseName, database, persistence };
  })().catch((error: unknown) => {
    handles.delete(databaseName);
    throw error;
  });
  handles.set(databaseName, opening);
  return opening;
}

async function closeHandle(handle: OfflinePersistenceHandle): Promise<void> {
  try {
    await handle.database.close?.();
  } catch {
    // A pagehide/worker teardown may already have closed the connection.
  }
}

/**
 * 前回のログアウトで消せなかった DB を、アプリ起動時に削除し直す。
 * 別タブがハンドルを持っている場合などは、また次の起動へ持ち越す。
 */
export async function retryPendingOfflineDeletion(): Promise<void> {
  const pending = readPendingOfflineDeletion();
  if (!pending) {
    return;
  }
  try {
    await removeOfflineDatabase(pending);
    clearPendingOfflineDeletion();
  } catch {
    // Keep the marker; the next startup retries after other tabs close.
  }
}

/** ログアウト時に、そのユーザーの DB と関連ファイルを削除する。 */
export async function removeOfflineDatabase(namespace: string): Promise<void> {
  const databaseName = offlineDatabaseName(namespace);
  const opening = handles.get(databaseName);
  handles.delete(databaseName);
  if (opening) {
    try {
      await closeHandle(await opening);
    } catch {
      // The database never opened; there is nothing to close.
    }
  }
  if (typeof navigator === "undefined" || typeof navigator.storage?.getDirectory !== "function") {
    return;
  }
  const root = await navigator.storage.getDirectory();
  for (const suffix of ["", "-journal", "-wal"]) {
    try {
      await root.removeEntry(`${databaseName}${suffix}`);
    } catch (error) {
      if ((error as { name?: string }).name !== "NotFoundError") {
        throw error;
      }
    }
  }
}
