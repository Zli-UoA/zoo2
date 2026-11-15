import type { AuthWorkerSession, AuthWorkerStore } from "./auth-worker-state";

const DATABASE = "zoo-oidc-auth";
const STORE = "sessions";

async function withStore<T>(
  mode: IDBTransactionMode,
  action: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    return await new Promise<T>((resolve, reject) => {
      const transaction = db.transaction(STORE, mode, { durability: "strict" });
      const request = action(transaction.objectStore(STORE));
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = () => reject(transaction.error);
      transaction.onerror = () => reject(transaction.error);
    });
  } finally {
    db.close();
  }
}

export const authWorkerStore: AuthWorkerStore = {
  staleSessionIds: (cutoff) => {
    const ids: string[] = [];
    return withStore("readonly", (store) => {
      const cursor = store.openCursor();
      cursor.onsuccess = () => {
        const row = cursor.result;
        if (!row) return;
        const value = row.value as AuthWorkerSession;
        if (
          value.state === "ready" &&
          (value.lastUsedAt ?? 0) <= cutoff &&
          typeof row.key === "string"
        )
          ids.push(row.key);
        row.continue();
      };
      return cursor;
    }).then(() => ids);
  },
  get: (id) =>
    withStore("readonly", (store) => store.get(id) as IDBRequest<AuthWorkerSession | undefined>),
  put: async (id, session) => {
    await withStore("readwrite", (store) => store.put(session, id));
  },
};
