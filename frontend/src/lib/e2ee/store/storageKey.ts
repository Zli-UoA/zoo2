/**
 * 暗号ストアの保存鍵（docs/e2ee.md の「暗号ストア」の保存鍵の置き場所）。
 *
 * 保存鍵は 32 バイトの乱数で、zoo-crypto が保存のたびにここから鍵を導く。保存鍵そのものは、
 * 抽出できない WebCrypto の AES-GCM 鍵（IndexedDB に保存する）で包んで、暗号ストアの `meta` に
 * 置く。ディスクやバックアップから暗号ストアのファイルだけを持ち出しても、IndexedDB の鍵を
 * 取り出せないので開けない。同じオリジンでコードを動かされると防げない（脅威モデル）。
 */

const IDB_NAME = "zoo-crypto-keys";
const IDB_STORE = "wrapping-keys";
/** `meta` に置く、包んだ保存鍵（`iv ‖ 暗号文` の base64）。 */
export const WRAPPED_STORAGE_KEY_META = "wrapped_storage_key";
const WRAP_AAD = new TextEncoder().encode("zoo e2ee storage key v1");

/** 包んだ保存鍵はあるが、開けない（IndexedDB の鍵が消えた、壊れた）。暗号ストアを作り直す。 */
export class StorageKeyUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StorageKeyUnavailableError";
  }
}

/** 包む鍵の置き場所。テストではメモリーにする。 */
export interface WrappingKeyStore {
  get(name: string): Promise<CryptoKey | undefined>;
  put(name: string, key: CryptoKey): Promise<void>;
  delete(name: string): Promise<void>;
}

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function openKeyDatabase(): Promise<IDBDatabase> {
  const req = indexedDB.open(IDB_NAME, 1);
  req.onupgradeneeded = () => req.result.createObjectStore(IDB_STORE);
  return request(req);
}

async function withStore<T>(
  mode: IDBTransactionMode,
  fn: (store: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = await openKeyDatabase();
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

/** IndexedDB の置き場所（`CryptoKey` は構造化複製でそのまま保存でき、抽出できないまま残る）。 */
export const indexedDbWrappingKeys: WrappingKeyStore = {
  get: (name) => withStore("readonly", (store) => store.get(name) as IDBRequest<CryptoKey>),
  put: async (name, key) => {
    await withStore("readwrite", (store) => store.put(key, name));
  },
  delete: async (name) => {
    await withStore("readwrite", (store) => store.delete(name));
  },
};

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(value: string): Uint8Array<ArrayBuffer> {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

/**
 * 暗号ストアの保存鍵を取り出す。なければ作り、包んだものを `saveWrapped` で `meta` に置く。
 *
 * - 包んだ保存鍵も包む鍵もない：新しく作る（新しい暗号ストア）。
 * - 包んだ保存鍵があるのに、包む鍵がない、または開けない：{@link StorageKeyUnavailableError}。
 *   暗号ストアのほかの値も読めないので、呼び出し側は作り直す。
 */
export async function loadStorageKey(options: {
  name: string;
  wrapped: string | null;
  saveWrapped: (wrapped: string) => Promise<void>;
  keys?: WrappingKeyStore;
}): Promise<Uint8Array> {
  const keys = options.keys ?? indexedDbWrappingKeys;
  let wrappingKey = await keys.get(options.name);
  if (options.wrapped) {
    if (!wrappingKey) {
      throw new StorageKeyUnavailableError("保存鍵を包む鍵がありません");
    }
    const packed = fromBase64(options.wrapped);
    try {
      const raw = await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: packed.subarray(0, 12), additionalData: WRAP_AAD },
        wrappingKey,
        packed.subarray(12),
      );
      return new Uint8Array(raw);
    } catch {
      throw new StorageKeyUnavailableError("保存鍵を開けません");
    }
  }
  if (!wrappingKey) {
    wrappingKey = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
      "encrypt",
      "decrypt",
    ]);
    await keys.put(options.name, wrappingKey);
  }
  const storageKey = crypto.getRandomValues(new Uint8Array(32));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const sealed = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv, additionalData: WRAP_AAD },
      wrappingKey,
      storageKey,
    ),
  );
  const packed = new Uint8Array(iv.length + sealed.length);
  packed.set(iv);
  packed.set(sealed, iv.length);
  await options.saveWrapped(toBase64(packed));
  return storageKey;
}
