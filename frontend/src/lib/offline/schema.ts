/**
 * 永続化フォーマットのバージョン。行の形やインデックスを変えたら上げる。
 * `sync-present-reset` により、古いキャッシュは Electric の再取得で置き換わる。
 */
export const OFFLINE_SCHEMA_VERSION = 1;
