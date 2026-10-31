import type { BrowserWASQLiteDatabase } from "@tanstack/browser-db-sqlite-persistence";

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
  const initialize = () =>
    (ready ??= database?.execute(
      "CREATE TABLE IF NOT EXISTS zoo_compose_shapes (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    ));
  return {
    async read(key) {
      if (!database) return memory.get(key) ?? null;
      try {
        await initialize();
        const rows = await database.execute<{ value: string }>(
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
        await database.execute(
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
