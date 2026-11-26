/**
 * 鍵バックアップからの取り込みの上限（docs/e2ee.md の「初回設定と解錠の実装」の Web）。
 * 画面のスレッド（`setupController.ts`）と Worker（`store/cryptoStore.ts`）の両方が使う。
 */

/**
 * 1 つの版で読むページの上限（1 ページ 1000 件で、100 万件）。サーバーが `next` を返し続けても、
 * 取り込みが終わらなくなったり、偽のセッションで保存先を使い切ったりしないようにする。
 */
export const BACKUP_IMPORT_MAX_PAGES = 1000;

/** 1 ページの行の上限（サーバーの `GET /api/e2ee/key_backup/keys` と同じ）。 */
export const BACKUP_IMPORT_PAGE_SIZE = 1000;
