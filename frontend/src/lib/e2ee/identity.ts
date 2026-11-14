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
/**
 * 読めなくなった（JSON でない）削除待ちの一覧を退避する先。元の値を残したまま、空の一覧から
 * 書き直して、以後の削除待ちを記録できるようにする。最後に退避した値だけを持つ。
 */
export const CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY =
  "zoo.e2ee.pendingDeviceDelete.unreadable";

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
  /**
   * 登録できたかを確かめられていない（登録の要求の結果を受け取っていない）。この端末の 404 は、
   * あとから届いた登録で端末ができうるので、すぐには消えたものとしない。
   */
  unconfirmed?: boolean;
  /** `unconfirmed` の端末で、続けて 404 を受け取った回数。 */
  notFound?: number;
}

/** 削除待ちの一覧を読めない（形式が違う）。一覧は書き換えずに残す。 */
export class PendingDeviceDeletionsUnreadableError extends Error {
  constructor() {
    super("削除待ちの端末の一覧を読めません");
  }
}

function sameDevice(a: PendingDeviceDeletion, b: PendingDeviceDeletion): boolean {
  return a.userId === b.userId && a.deviceId === b.deviceId;
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
 * 削除待ちを上書きしない。同じ端末があれば、`unconfirmed` を合わせる（どちらかが真なら真）。
 *
 * 一覧を読めなければ、元の値を `CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY` に退避してから、空の
 * 一覧に足す（`readForUpdate`）。
 */
export function markDeviceDeletionPending(pending: PendingDeviceDeletion): void {
  const all = readForUpdate();
  if (!all) return;
  const existing = all.find((entry) => sameDevice(entry, pending));
  const merged = normalize({
    ...pending,
    unconfirmed: Boolean(existing?.unconfirmed || pending.unconfirmed),
    notFound: existing?.notFound ?? 0,
  });
  const next = existing
    ? all.map((entry) => (entry === existing ? merged : entry))
    : [...all, merged];
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, JSON.stringify(next));
}

/** 同じ端末の記録を `pending` に置き換える（404 の回数を数えるため）。なければ何もしない。 */
export function updatePendingDeviceDeletion(pending: PendingDeviceDeletion): void {
  const all = readForUpdate();
  if (!all) return;
  if (!all.some((entry) => sameDevice(entry, pending))) return;
  const next = all.map((entry) => (sameDevice(entry, pending) ? normalize(pending) : entry));
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, JSON.stringify(next));
}

/**
 * 書き換えるために一覧を読む。読めない値は別のキーに退避して、空の一覧から始める（読めない値を
 * 残しても消し直しには使えず、以後の削除待ちを記録できなくなるため）。退避できなければ `null`
 * （元の値を失わないよう、書き換えない）。
 */
function readForUpdate(): PendingDeviceDeletion[] | null {
  try {
    return readPendingDeviceDeletions();
  } catch {
    const raw = readStorageValue(CRYPTO_PENDING_DEVICE_DELETE_KEY);
    try {
      if (raw) window.localStorage.setItem(CRYPTO_PENDING_DEVICE_DELETE_UNREADABLE_KEY, raw);
      window.localStorage.removeItem(CRYPTO_PENDING_DEVICE_DELETE_KEY);
      return [];
    } catch {
      return null;
    }
  }
}

function normalize(entry: PendingDeviceDeletion): PendingDeviceDeletion {
  const { userId, deviceId } = entry;
  const notFound = entry.notFound ?? 0;
  return {
    userId,
    deviceId,
    ...(entry.unconfirmed ? { unconfirmed: true } : {}),
    ...(notFound > 0 ? { notFound } : {}),
  };
}

/**
 * 削除待ちの端末（全ユーザー分）。読めない（JSON でない、形式が違う）ときは
 * `PendingDeviceDeletionsUnreadableError` を投げる。値は、次に書き換えるときに退避する。
 */
export function readPendingDeviceDeletions(): PendingDeviceDeletion[] {
  const raw = readStorageValue(CRYPTO_PENDING_DEVICE_DELETE_KEY);
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new PendingDeviceDeletionsUnreadableError();
  }
  // 前の形式（1 件のオブジェクト）も読む。形式が違う値（`null`、数、`user_id` のキーなど）は、
  // 読み飛ばして空の一覧で上書きしないよう、読めないものとして扱う（TUI、Android と同じ）。
  const entries = Array.isArray(parsed) ? parsed : [parsed];
  return entries.map((entry) => {
    const { userId, deviceId, unconfirmed, notFound } = (
      typeof entry === "object" && entry !== null ? entry : {}
    ) as Partial<PendingDeviceDeletion>;
    if (typeof userId !== "string" || typeof deviceId !== "string") {
      throw new PendingDeviceDeletionsUnreadableError();
    }
    return normalize({
      userId,
      deviceId,
      unconfirmed: unconfirmed === true,
      notFound:
        typeof notFound === "number" && Number.isInteger(notFound) && notFound > 0 ? notFound : 0,
    });
  });
}

/** 消せた端末を、削除待ちから外す。 */
export function clearPendingDeviceDeletion(done: PendingDeviceDeletion): void {
  const all = readForUpdate();
  if (!all) return;
  const rest = all.filter((entry) => !sameDevice(entry, done));
  if (rest.length === all.length) return;
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, rest.length > 0 ? JSON.stringify(rest) : null);
}
