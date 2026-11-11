import { offlineDatabaseName } from "./identity";

/**
 * チャンネルの最新ページの控え (オフライン表示用)。
 *
 * メッセージのコレクションは on-demand で同期するため、永続化層は、保存済みの行を
 * 「未認定」として隔離し、新しい subset snapshot でしか使わない。オフラインで再読み込みすると
 * 本文が出ないので、画面に出した最新ページを、別に IndexedDB へ控えておく。
 *
 * ユーザーごとの名前空間で DB を分け、ログアウト時に `deleteFeedCache` で消す (OPFS の DB と同じ扱い)。
 */
const STORE = "feeds";
const VERSION = 1;

/**
 * ログアウトで消した名前空間。画面が残っていて、予約済みの保存が後から走っても、
 * 消した DB を作り直さないようにする (本文が端末に残るため)。名前空間が有効になったら外す。
 */
const blocked = new Set<string>();

/** ログアウトの削除に入る前に呼ぶ。以後、その名前空間への保存を拒否する。 */
export function blockFeedCache(namespace: string): void {
  blocked.add(namespace);
}

/** 名前空間が (ログイン済みで) 使われ始めたら呼ぶ。保存を再開する。 */
export function enableFeedCache(namespace: string): void {
  blocked.delete(namespace);
}

export interface FeedCacheEntry<Row> {
  channelId: string;
  savedAt: number;
  rows: Row[];
}

export function feedCacheDatabaseName(namespace: string): string {
  return `${offlineDatabaseName(namespace)}.feed`;
}

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function open(namespace: string): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(feedCacheDatabaseName(namespace), VERSION);
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE, { keyPath: "channelId" });
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
    req.onblocked = () => reject(new Error("feed cache is blocked"));
  });
}

/** 画面に出している行のうち、新しい方から `limit` 件を控える。失敗は無視する (控えは、なくても動く)。 */
export async function saveFeedCache<Row>(
  namespace: string,
  channelId: string,
  rows: readonly Row[],
  limit: number,
): Promise<void> {
  if (typeof indexedDB === "undefined" || rows.length === 0 || blocked.has(namespace)) return;
  try {
    const db = await open(namespace);
    try {
      // 開いている間にログアウトの削除が始まった場合は、書かない。
      if (blocked.has(namespace)) return;
      // 構造化複製に載らない値 (関数など) が混ざらないよう、JSON を経由する。
      const entry: FeedCacheEntry<Row> = {
        channelId,
        savedAt: Date.now(),
        rows: JSON.parse(JSON.stringify(rows.slice(-limit))) as Row[],
      };
      await request(db.transaction(STORE, "readwrite").objectStore(STORE).put(entry));
    } finally {
      db.close();
    }
  } catch {
    // 容量不足・ブロックなど。次の機会に控え直す。
  }
}

export async function loadFeedCache<Row>(
  namespace: string,
  channelId: string,
): Promise<FeedCacheEntry<Row> | null> {
  if (typeof indexedDB === "undefined") return null;
  try {
    const db = await open(namespace);
    try {
      const entry = await request(
        db.transaction(STORE, "readonly").objectStore(STORE).get(channelId),
      );
      return (entry as FeedCacheEntry<Row> | undefined) ?? null;
    } finally {
      db.close();
    }
  } catch {
    return null;
  }
}

/** ログアウト時に、そのユーザーの控えを消す。 */
export function deleteFeedCache(namespace: string): Promise<void> {
  if (typeof indexedDB === "undefined") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const req = indexedDB.deleteDatabase(feedCacheDatabaseName(namespace));
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
    req.onblocked = () => resolve();
  });
}

/** そのチャンネルの控えを消す (全件が削除されて、空になったときなど)。 */
export async function clearFeedCache(namespace: string, channelId: string): Promise<void> {
  if (typeof indexedDB === "undefined") return;
  try {
    const db = await open(namespace);
    try {
      await request(db.transaction(STORE, "readwrite").objectStore(STORE).delete(channelId));
    } finally {
      db.close();
    }
  } catch {
    // 次の機会に消す。
  }
}
