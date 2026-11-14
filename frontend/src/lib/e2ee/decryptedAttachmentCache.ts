import type { AttachmentInfo } from "@zoo/crypto-wasm";

import { EncryptedAttachmentError } from "./attachments";

/**
 * 復号した添付（#364）の、上限付きのキャッシュと実行中の要求。
 *
 * メッセージ一覧は仮想化されていて、画面外に出た行はアンマウントされる。戻るたびに `/url` の取得、暗号文の
 * ダウンロード、Worker での復号をやり直さないように、復号した `Blob` をここに残す。object URL は表示する
 * コンポーネントごとに作って捨てる（`Blob` は複製しないので安い）。
 *
 * キーは添付の ID だけでなく、本文の鍵、IV、ハッシュ、MIME を含める。同じ `attachment_id` を別の鍵で
 * 指すメッセージに、ほかのメッセージの平文を出さないため。
 *
 * 復号の失敗（ハッシュや大きさが合わない）は覚えておき、再マウントのたびに全量を取り直さない。やり直すのは
 * 利用者が再試行を押したとき（`retry`）だけ。取得の失敗（通信、期限切れ）は覚えない。
 */
export interface DecryptedAttachmentCache {
  /** キャッシュにある復号済みの `Blob`。 */
  peek(info: AttachmentInfo): Blob | undefined;
  /** 復号に失敗したと覚えている添付か。 */
  failed(info: AttachmentInfo): boolean;
  /**
   * キャッシュになければ `loader` で取得して復号する。同じ添付の実行中の要求はまとめる。
   * 復号に失敗したと覚えている添付は、`retry` でなければ取得せずに失敗を返す。
   */
  load(
    info: AttachmentInfo,
    loader: (info: AttachmentInfo) => Promise<Blob>,
    options?: { retry?: boolean },
  ): Promise<Blob>;
}

/** 平文を置いておく量の上限（既定）。添付 1 件の上限（50 MiB）より大きくする。 */
export const DECRYPTED_ATTACHMENT_CACHE_MAX_BYTES = 64 * 1024 * 1024;
/** 置いておく件数の上限（既定）。 */
export const DECRYPTED_ATTACHMENT_CACHE_MAX_ENTRIES = 100;

export function decryptedAttachmentCacheKey(info: AttachmentInfo): string {
  const { file } = info;
  return [info.attachment_id, file.key.k, file.iv, file.hashes.sha256, info.mimetype].join("\n");
}

export function createDecryptedAttachmentCache({
  maxBytes = DECRYPTED_ATTACHMENT_CACHE_MAX_BYTES,
  maxEntries = DECRYPTED_ATTACHMENT_CACHE_MAX_ENTRIES,
}: { maxBytes?: number; maxEntries?: number } = {}): DecryptedAttachmentCache {
  // Map は入れた順に並ぶ。使うたびに入れ直して、先頭を最も古いものにする（LRU）。
  const entries = new Map<string, Blob>();
  const pending = new Map<string, Promise<Blob>>();
  const failures = new Map<string, unknown>();
  let totalBytes = 0;

  function remember(key: string, blob: Blob) {
    if (blob.size > maxBytes) return;
    const previous = entries.get(key);
    if (previous) {
      entries.delete(key);
      totalBytes -= previous.size;
    }
    entries.set(key, blob);
    totalBytes += blob.size;
    for (const [oldKey, oldBlob] of entries) {
      if (totalBytes <= maxBytes && entries.size <= maxEntries) break;
      entries.delete(oldKey);
      totalBytes -= oldBlob.size;
    }
  }

  return {
    peek(info) {
      const key = decryptedAttachmentCacheKey(info);
      const blob = entries.get(key);
      if (blob) {
        entries.delete(key);
        entries.set(key, blob);
      }
      return blob;
    },
    failed(info) {
      return failures.has(decryptedAttachmentCacheKey(info));
    },
    load(info, loader, options = {}) {
      const cached = this.peek(info);
      if (cached) return Promise.resolve(cached);
      const key = decryptedAttachmentCacheKey(info);
      if (failures.has(key)) {
        if (!options.retry) return Promise.reject(failures.get(key));
        failures.delete(key);
      }
      let request = pending.get(key);
      if (!request) {
        request = loader(info)
          .then(
            (blob) => {
              remember(key, blob);
              return blob;
            },
            (error: unknown) => {
              if (error instanceof EncryptedAttachmentError && error.reason === "decrypt") {
                failures.set(key, error);
                // 覚える件数も上限までにする（古いものから忘れる）。
                for (const oldKey of failures.keys()) {
                  if (failures.size <= maxEntries) break;
                  failures.delete(oldKey);
                }
              }
              throw error;
            },
          )
          .finally(() => pending.delete(key));
        pending.set(key, request);
      }
      return request;
    },
  };
}

const caches = new WeakMap<object, DecryptedAttachmentCache>();

/**
 * `scope` ごとのキャッシュ。暗号の Worker を渡すと、ロックやログアウトで Worker を捨てたときに、
 * 平文もまとめて手放す。
 */
export function decryptedAttachmentCacheFor(scope: object): DecryptedAttachmentCache {
  let cache = caches.get(scope);
  if (!cache) caches.set(scope, (cache = createDecryptedAttachmentCache()));
  return cache;
}
