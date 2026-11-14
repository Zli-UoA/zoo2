import { attachmentUrlExpiresAt, shouldRefreshAttachmentUrl } from "../attachments";
import type { ApiClient } from "./client";
import type { AttachmentUrlResponse } from "./types";

/**
 * 添付の署名付き URL（`GET /api/attachments/{id}/url`）のキャッシュと、実行中の要求。
 *
 * `scope`（接続の設定）ごとに分け、同じ添付の URL の取得を 1 回にまとめる（docs/request-design.md）。
 * 平文の添付（`useAttachmentUrl`）と暗号化した添付（#364）で共有する。
 */
export type AttachmentUrlEntry = { url: string; expiresAt: number };

const urlCaches = new WeakMap<object, Map<string, AttachmentUrlEntry>>();
const urlRequests = new WeakMap<object, Map<string, Promise<AttachmentUrlEntry>>>();

function scoped<T>(maps: WeakMap<object, Map<string, T>>, scope: object): Map<string, T> {
  let map = maps.get(scope);
  if (!map) maps.set(scope, (map = new Map()));
  return map;
}

/** キャッシュにある URL（期限が近くても返す）。 */
export function cachedAttachmentUrl(scope: object, attachmentId: string) {
  return scoped(urlCaches, scope).get(attachmentId);
}

/** キャッシュの URL を捨てる（取得に失敗したときなど）。 */
export function forgetAttachmentUrl(scope: object, attachmentId: string) {
  scoped(urlCaches, scope).delete(attachmentId);
}

/** URL を取得し直す。同じ添付の取得が実行中なら、それを待つ。 */
export function requestAttachmentUrl(
  client: ApiClient,
  scope: object,
  attachmentId: string,
): Promise<AttachmentUrlEntry> {
  const requests = scoped(urlRequests, scope);
  let pending = requests.get(attachmentId);
  if (!pending) {
    pending = client
      .requestWithTx<AttachmentUrlResponse>(
        `/api/attachments/${encodeURIComponent(attachmentId)}/url`,
        { method: "GET" },
      )
      .then(({ data }) => {
        const value = {
          url: data.download_url,
          expiresAt: attachmentUrlExpiresAt(data.download_url),
        };
        scoped(urlCaches, scope).set(attachmentId, value);
        return value;
      })
      .finally(() => requests.delete(attachmentId));
    requests.set(attachmentId, pending);
  }
  return pending;
}

/** 期限まで余裕のある URL はキャッシュから返し、なければ取得する。 */
export async function resolveAttachmentUrl(
  client: ApiClient,
  scope: object,
  attachmentId: string,
): Promise<string> {
  const cached = cachedAttachmentUrl(scope, attachmentId);
  if (cached && !shouldRefreshAttachmentUrl(cached.expiresAt)) return cached.url;
  return (await requestAttachmentUrl(client, scope, attachmentId)).url;
}
