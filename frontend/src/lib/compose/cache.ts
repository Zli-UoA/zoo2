import type { BrowserWASQLiteDatabase } from "@tanstack/browser-db-sqlite-persistence";

// The OPFS worker accepts concurrent messages, but SQLite statements sharing
// one connection must finish before the next statement starts.
const databaseQueues = new WeakMap<BrowserWASQLiteDatabase, Promise<void>>();

function execute<T>(database: BrowserWASQLiteDatabase, sql: string, params?: readonly unknown[]) {
  const previous = databaseQueues.get(database) ?? Promise.resolve();
  const result = previous.then(() => database.execute<T>(sql, params));
  databaseQueues.set(
    database,
    result.then(
      () => {},
      () => {},
    ),
  );
  return result;
}

export interface ComposeCache {
  read: (key: string) => Promise<string | null>;
  write: (key: string, value: string) => Promise<void>;
}

/** Use the same user-isolated OPFS connection and logout deletion as the Web frontend. */
export function createComposeCache(
  database: BrowserWASQLiteDatabase | null,
  onError?: (error: unknown) => void,
): ComposeCache {
  const memory = new Map<string, string>();
  let ready: Promise<unknown> | undefined;
  const initialize = () => {
    if (!database) return;
    return (ready ??= execute(
      database,
      "CREATE TABLE IF NOT EXISTS zoo_compose_shapes (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    ));
  };
  return {
    async read(key) {
      if (!database) return memory.get(key) ?? null;
      try {
        await initialize();
        const rows = await execute<{ value: string }>(
          database,
          "SELECT value FROM zoo_compose_shapes WHERE key = ?",
          [key],
        );
        return rows[0]?.value ?? null;
      } catch (error) {
        onError?.(error);
        throw error;
      }
    },
    async write(key, value) {
      if (!database) {
        memory.set(key, value);
        return;
      }
      try {
        await initialize();
        await execute(
          database,
          "INSERT INTO zoo_compose_shapes (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
          [key, value],
        );
      } catch (error) {
        onError?.(error);
        throw error;
      }
    },
  };
}
