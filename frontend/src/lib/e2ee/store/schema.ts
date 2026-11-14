/**
 * 暗号ストアの SQLite の表（docs/e2ee.md の「暗号ストア」）。Worker の `sqliteBackend.ts` と、
 * Push を復号する Service Worker が読む写し（`lib/e2ee/push/snapshotBackend.ts`）で共有する。
 */

/** 表の形を変えたら上げる。古い版の移行は、必要になった版で足す。 */
export const CRYPTO_SCHEMA_VERSION = 1;

export const CRYPTO_SCHEMA = `
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS account (id INTEGER PRIMARY KEY CHECK (id = 1), pickle TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS olm_sessions (
  their_identity_key TEXT NOT NULL,
  session_id TEXT NOT NULL,
  pickle TEXT NOT NULL,
  PRIMARY KEY (their_identity_key, session_id)
);
CREATE TABLE IF NOT EXISTS inbound_sessions (
  key TEXT PRIMARY KEY,
  pickle TEXT NOT NULL,
  backed_up INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS trust_store (id INTEGER PRIMARY KEY CHECK (id = 1), pickle TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS processed_to_device (id TEXT PRIMARY KEY);
PRAGMA user_version = ${CRYPTO_SCHEMA_VERSION};
`;
