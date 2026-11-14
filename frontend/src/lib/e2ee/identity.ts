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
 * 一覧を読めなければ書かない（空の一覧で上書きして、ほかの削除待ちを失わないため）。
 */
export function markDeviceDeletionPending(pending: PendingDeviceDeletion): void {
  let all: PendingDeviceDeletion[];
  try {
    all = readPendingDeviceDeletions();
  } catch {
    return;
  }
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
  let all: PendingDeviceDeletion[];
  try {
    all = readPendingDeviceDeletions();
  } catch {
    return;
  }
  if (!all.some((entry) => sameDevice(entry, pending))) return;
  const next = all.map((entry) => (sameDevice(entry, pending) ? normalize(pending) : entry));
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, JSON.stringify(next));
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
 * 削除待ちの端末（全ユーザー分）。読めない（JSON でない）ときは
 * `PendingDeviceDeletionsUnreadableError` を投げ、一覧はそのまま残す。
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
  // 前の形式（1 件のオブジェクト）も読む。
  const entries = Array.isArray(parsed) ? parsed : [parsed];
  return entries.flatMap((entry) => {
    const { userId, deviceId, unconfirmed, notFound } = (entry ??
      {}) as Partial<PendingDeviceDeletion>;
    if (typeof userId !== "string" || typeof deviceId !== "string") return [];
    return [
      normalize({
        userId,
        deviceId,
        unconfirmed: unconfirmed === true,
        notFound:
          typeof notFound === "number" && Number.isInteger(notFound) && notFound > 0 ? notFound : 0,
      }),
    ];
  });
}

/** 消せた端末を、削除待ちから外す。一覧を読めなければ何もしない。 */
export function clearPendingDeviceDeletion(done: PendingDeviceDeletion): void {
  let all: PendingDeviceDeletion[];
  try {
    all = readPendingDeviceDeletions();
  } catch {
    return;
  }
  const rest = all.filter((entry) => !sameDevice(entry, done));
  if (rest.length === all.length) return;
  writeStorage(CRYPTO_PENDING_DEVICE_DELETE_KEY, rest.length > 0 ? JSON.stringify(rest) : null);
}
