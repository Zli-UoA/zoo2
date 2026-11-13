import { hashedDatabaseName } from "../offline/identity";
import { readStorageValue } from "../storage";

/**
 * 暗号ストアの名前と、ログアウトで消し切れなかったものの印（docs/e2ee.md の「暗号ストア」）。
 *
 * 暗号ストアはオフライン DB（`zoo-offline-*`）とは別のファイルにする。`OFFLINE_SCHEMA_VERSION`
 * を変えてオフライン DB を作り直しても、端末の鍵は消えない。名前空間はオフライン DB と同じ
 * （`resolveOfflineNamespace`）で、ユーザーごとに別のファイルになる。
 */
export const CRYPTO_DATABASE_PREFIX = "zoo-crypto-";
/** ログアウトで消せなかった暗号ストアの名前空間。次の起動で消し直す。 */
export const CRYPTO_PENDING_DELETE_KEY = "zoo.e2ee.pendingStoreDelete";
/** 削除 API が失敗した端末（`{userId, deviceId}`）。同じユーザーの次のログインで削除し直す。 */
export const CRYPTO_PENDING_DEVICE_DELETE_KEY = "zoo.e2ee.pendingDeviceDelete";

export function cryptoDatabaseName(namespace: string): string {
  return hashedDatabaseName(CRYPTO_DATABASE_PREFIX, namespace);
}

/** 1 つのタブだけが暗号ストアを開くための Web Locks の名前。 */
export function cryptoLockName(namespace: string): string {
  return `zoo-crypto:${cryptoDatabaseName(namespace)}`;
}

export interface PendingDeviceDeletion {
  userId: string;
  deviceId: string;
}

function writeStorage(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // Storage may be blocked; the retry is best effort.
  }
}

export function markCryptoStoreDeletionPending(namespace: string): void {
  writeStorage(CRYPTO_PENDING_DELETE_KEY, namespace);
}

export function readPendingCryptoStoreDeletion(): string | null {
  return readStorageValue(CRYPTO_PENDING_DELETE_KEY) || null;
}

export function clearPendingCryptoStoreDeletion(): void {
  writeStorage(CRYPTO_PENDING_DELETE_KEY, null);
}

/**
 * 削除 API の結果を待たずに暗号ストアを消すので、失敗に備えて先に残す。`device_id` は秘密ではない
 * （docs/e2ee.md の「ログアウトと端末の削除」）。ユーザーと端末ごとに残し、別のユーザーの
 * 削除待ちを上書きしない。
 */
export function markDeviceDeletionPending(pending: PendingDeviceDeletion): void {
  const all = readPendingDeviceDeletions().filter(
    (entry) => entry.userId !== pending.userId || entry.deviceId !== pending.deviceId,
  );
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, JSON.stringify([...all, pending]));
}

/** 削除待ちの端末（全ユーザー分）。 */
export function readPendingDeviceDeletions(): PendingDeviceDeletion[] {
  const raw = readStorageValue(CRYPTO_PENDING_DEVICE_DELETE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    // 前の形式（1 件のオブジェクト）も読む。
    const entries = Array.isArray(parsed) ? parsed : [parsed];
    return entries.flatMap((entry) => {
      const { userId, deviceId } = (entry ?? {}) as Partial<PendingDeviceDeletion>;
      return typeof userId === "string" && typeof deviceId === "string"
        ? [{ userId, deviceId }]
        : [];
    });
  } catch {
    writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, null);
    return [];
  }
}

/** 消せた端末を、削除待ちから外す。 */
export function clearPendingDeviceDeletion(done: PendingDeviceDeletion): void {
  const rest = readPendingDeviceDeletions().filter(
    (entry) => entry.userId !== done.userId || entry.deviceId !== done.deviceId,
  );
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, rest.length > 0 ? JSON.stringify(rest) : null);
}
