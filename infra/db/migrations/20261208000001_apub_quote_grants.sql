-- 引用元ノートと引用権限のテーブル。
-- もとは main の 20261205000002_apub_quote_grants.sql だが、このブランチでは 20261208000001 に改番している。
-- 古い preview DB は既に 20261208 系のマイグレーションを適用済みで、後から 20261205000002 を追加すると
-- Atlas が out-of-order として適用を拒否するため、ファイル名は 20261208000001 のまま維持する。
-- BASE 適用済み DB では main 側のマイグレーションでテーブル・インデックス・トリガーが既に作られている。
-- 同じ DDL を再実行しても停止しないよう、作成対象はすべて冪等にしている。
-- Create "apub_quote_originals" table
CREATE TABLE IF NOT EXISTS "apub_quote_originals" (
  "id" text NOT NULL,
  "actor_id" text NOT NULL,
  "content" text NOT NULL,
  "published" timestamptz NOT NULL,
  "updated_at" timestamptz NOT NULL,
  "superseded" boolean NOT NULL DEFAULT false,
  "superseded_by_public" boolean NOT NULL DEFAULT false,
  "observed_audience" text NULL,
  "revision" bigint NOT NULL DEFAULT 1,
  "parent_id" text NULL,
  "emojis" jsonb NOT NULL,
  "media" jsonb NOT NULL,
  PRIMARY KEY ("id"),
  CONSTRAINT "apub_quote_originals_actor_id_fkey" FOREIGN KEY ("actor_id") REFERENCES "apub_actors" ("id") ON UPDATE NO ACTION ON DELETE NO ACTION
);
-- Create index "apub_quote_originals_actor" to table: "apub_quote_originals"
CREATE INDEX IF NOT EXISTS "apub_quote_originals_actor" ON "apub_quote_originals" ("actor_id");
-- Create "apub_quote_grants" table
CREATE TABLE IF NOT EXISTS "apub_quote_grants" (
  "post_id" text NOT NULL,
  "user_id" text NOT NULL,
  "note_revision" bigint NOT NULL,
  PRIMARY KEY ("post_id", "user_id"),
  CONSTRAINT "apub_quote_grants_post_id_fkey" FOREIGN KEY ("post_id") REFERENCES "apub_quote_originals" ("id") ON UPDATE NO ACTION ON DELETE CASCADE,
  CONSTRAINT "apub_quote_grants_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
-- Create index "apub_quote_grants_user" to table: "apub_quote_grants"
CREATE INDEX IF NOT EXISTS "apub_quote_grants_user" ON "apub_quote_grants" ("user_id");

-- Synchronize recipient ordering only if an ordering value has changed.
DROP TRIGGER IF EXISTS apub_post_recipient_order ON apub_remote_posts;
CREATE TRIGGER apub_post_recipient_order AFTER UPDATE OF actor_id,published,parent_id,deleted_at ON apub_remote_posts FOR EACH ROW
  WHEN ((OLD.actor_id,OLD.published,OLD.parent_id,OLD.deleted_at IS NOT NULL) IS DISTINCT FROM (NEW.actor_id,NEW.published,NEW.parent_id,NEW.deleted_at IS NOT NULL))
  EXECUTE FUNCTION sync_apub_recipient_order();

-- BASE 適用済み DB では同名トリガーが既に存在するため、作り直してから登録する。
DROP TRIGGER IF EXISTS apub_quote_grants_home_live ON apub_quote_grants;
CREATE TRIGGER apub_quote_grants_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_quote_grants
  FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
