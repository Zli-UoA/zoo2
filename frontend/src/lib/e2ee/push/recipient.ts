/**
 * このブラウザーで今ログインしているユーザー（Push を復号してよい宛先）。
 *
 * Service Worker は localStorage も画面の状態も読めないので、タブ（`E2eeProvider`）が IndexedDB に
 * 置き、ログアウトで消す。Service Worker は、Push の `recipient_id` がこれと一致するときだけ復号する
 * （docs/e2ee.md の「Web Push の復号」）。ログアウトせずに別のユーザーでログインし直しても、
 * 前のユーザー宛ての Push の本文を、今のユーザーの使っている画面に出さない。
 *
 * 秘密ではない（ユーザー ID だけ）。
 */

const IDB_NAME = "zoo-push";
const IDB_STORE = "state";
const KEY = "recipient";

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function withStore<T>(
  mode: IDBTransactionMode,
  fn: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const open = indexedDB.open(IDB_NAME, 1);
  open.onupgradeneeded = () => open.result.createObjectStore(IDB_STORE);
  const db = await request(open);
  try {
    const tx = db.transaction(IDB_STORE, mode);
    const result = await request(fn(tx.objectStore(IDB_STORE)));
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
    return result;
  } finally {
    db.close();
  }
}

export async function rememberPushRecipient(userId: string): Promise<void> {
  await withStore("readwrite", (store) => store.put(userId, KEY));
}

export async function forgetPushRecipient(): Promise<void> {
  await withStore("readwrite", (store) => store.delete(KEY));
}

export async function readPushRecipient(): Promise<string | null> {
  const value = await withStore("readonly", (store) => store.get(KEY) as IDBRequest<unknown>);
  return typeof value === "string" ? value : null;
}
