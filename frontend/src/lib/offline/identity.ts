import { readStorageValue } from "../storage";

/** 最後に解決したユーザー。オフライン起動時の OPFS 名前空間に使う。 */
export const OFFLINE_LAST_USER_KEY = "zoo.offline.lastUser";
export const OFFLINE_DATABASE_PREFIX = "zoo-offline-";

/** FNV-1a を 2 種類の seed で回し、短く衝突しにくい DB 名にする。 */
function hashNamespace(namespace: string, seed: number): string {
  let hash = seed >>> 0;
  for (let index = 0; index < namespace.length; index += 1) {
    hash ^= namespace.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

/**
 * OPFS のデータベース名に使うユーザー名前空間。
 *
 * 1つの DB をユーザー間で共有せず、ユーザーごとに物理的に分ける。
 * ログアウト時は該当 DB を削除でき、別ユーザーが同じ端末を使っても
 * 前のユーザーのメッセージは読めない。
 *
 * wa-sqlite の OPFS VFS は長いファイル名で open に失敗するため、
 * ユーザー ID はそのまま使わず固定長のハッシュにする。
 */
export function offlineDatabaseName(namespace: string): string {
  const left = hashNamespace(namespace, 0x811c9dc5);
  const right = hashNamespace(namespace, 0x9e3779b9);
  return `${OFFLINE_DATABASE_PREFIX}${left}${right}.sqlite`;
}

/**
 * 名前空間を決める。dev / preview は固定ユーザー、OIDC は解決済みの
 * 現在ユーザー、解決前・オフラインは最後に使ったユーザーへフォールバックする。
 */
export function resolveOfflineNamespace(input: {
  devUserId: string | null;
  currentUserId: string | null;
}): string {
  if (input.devUserId) {
    return `dev-${input.devUserId}`;
  }
  if (input.currentUserId) {
    return `user-${input.currentUserId}`;
  }
  const last = readLastOfflineUserId();
  return last ? `user-${last}` : "anonymous";
}

export function readLastOfflineUserId(): string | null {
  return readStorageValue(OFFLINE_LAST_USER_KEY) || null;
}

export function rememberOfflineUserId(userId: string): void {
  try {
    window.localStorage.setItem(OFFLINE_LAST_USER_KEY, userId);
  } catch {
    // Storage may be blocked; the current session still works in memory.
  }
}

export function forgetOfflineUserId(userId?: string | null): void {
  try {
    if (!userId || window.localStorage.getItem(OFFLINE_LAST_USER_KEY) === userId) {
      window.localStorage.removeItem(OFFLINE_LAST_USER_KEY);
    }
  } catch {
    // Storage may be blocked; nothing to clear.
  }
}
