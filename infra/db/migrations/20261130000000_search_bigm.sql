-- atlas:txmode none

-- 全文検索のための拡張とインデックス。pg_bigm は本番では CNPG の Database リソースが先に作る
-- （Zli-UoA/zoo2-infra）。作成済みなら、スーパーユーザーでない zoo2 ロールでも IF NOT EXISTS は通る。
CREATE EXTENSION IF NOT EXISTS pg_bigm WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS btree_gin WITH SCHEMA public;
-- 書き込みを止めないよう CONCURRENTLY で作る。fastupdate = off で pending list を持たない（検索のたびに全体を読むため）。
--
-- CONCURRENTLY が途中で失敗すると INVALID の索引が残り、IF NOT EXISTS は名前だけを見て飛ばしてしまう。
-- txmode none の Atlas は、再実行すると失敗した文から続ける（成功した前の文は流さない）。そのため、作る文の
-- 後にも INVALID を消すブロックを置き、もう一度作る。
--   1. 最初の作成が失敗 → 再実行は作成から（INVALID なので飛ばす）→ 下の DO が INVALID を消す → 2 回目の作成で作る。
--   2. 2 回目の作成も失敗したら、最後の DO が止める（INVALID のまま黙って通さない）。
-- 正常なら、後ろの DO と 2 回目の作成は何もしない。DROP は lock_timeout を付けて、表のロックを待ちすぎない。
-- （DO の中で RAISE すると DROP も取り消されるので、消すブロックでは止めない。）
DO $$
DECLARE name text;
BEGIN
  PERFORM set_config('lock_timeout', '10s', true);
  FOREACH name IN ARRAY ARRAY['messages_search_month', 'attachments_search_month', 'channel_notes_search_month', 'messages_search_reply_floor'] LOOP
    IF EXISTS (SELECT 1 FROM pg_index WHERE indexrelid = to_regclass(name) AND NOT indisvalid) THEN
      EXECUTE format('DROP INDEX %I', name);
    END IF;
  END LOOP;
END $$;
CREATE INDEX CONCURRENTLY IF NOT EXISTS "messages_search_month" ON "messages" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(content) public.gin_bigm_ops) WITH (fastupdate = off) WHERE ((deleted_at IS NULL) AND (content <> ''::text));
CREATE INDEX CONCURRENTLY IF NOT EXISTS "attachments_search_month" ON "attachments" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(file_name) public.gin_bigm_ops) WITH (fastupdate = off) WHERE (message_id IS NOT NULL);
CREATE INDEX CONCURRENTLY IF NOT EXISTS "channel_notes_search_month" ON "channel_notes" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower((((title)::text || E'\n') || content)) public.gin_bigm_ops) WITH (fastupdate = off) WHERE (deleted_at IS NULL);
CREATE INDEX CONCURRENTLY IF NOT EXISTS "messages_search_reply_floor" ON "messages" ("channel_id", "created_at") WHERE ((parent_id IS NOT NULL) AND (deleted_at IS NULL));
DO $$
DECLARE name text;
BEGIN
  PERFORM set_config('lock_timeout', '10s', true);
  FOREACH name IN ARRAY ARRAY['messages_search_month', 'attachments_search_month', 'channel_notes_search_month', 'messages_search_reply_floor'] LOOP
    IF EXISTS (SELECT 1 FROM pg_index WHERE indexrelid = to_regclass(name) AND NOT indisvalid) THEN
      EXECUTE format('DROP INDEX %I', name);
    END IF;
  END LOOP;
END $$;
CREATE INDEX CONCURRENTLY IF NOT EXISTS "messages_search_month" ON "messages" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(content) public.gin_bigm_ops) WITH (fastupdate = off) WHERE ((deleted_at IS NULL) AND (content <> ''::text));
CREATE INDEX CONCURRENTLY IF NOT EXISTS "attachments_search_month" ON "attachments" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(file_name) public.gin_bigm_ops) WITH (fastupdate = off) WHERE (message_id IS NOT NULL);
CREATE INDEX CONCURRENTLY IF NOT EXISTS "channel_notes_search_month" ON "channel_notes" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower((((title)::text || E'\n') || content)) public.gin_bigm_ops) WITH (fastupdate = off) WHERE (deleted_at IS NULL);
CREATE INDEX CONCURRENTLY IF NOT EXISTS "messages_search_reply_floor" ON "messages" ("channel_id", "created_at") WHERE ((parent_id IS NOT NULL) AND (deleted_at IS NULL));
DO $$
DECLARE name text;
BEGIN
  FOREACH name IN ARRAY ARRAY['messages_search_month', 'attachments_search_month', 'channel_notes_search_month', 'messages_search_reply_floor'] LOOP
    IF NOT EXISTS (SELECT 1 FROM pg_index WHERE indexrelid = to_regclass(name) AND indisvalid) THEN
      RAISE EXCEPTION 'search index % is missing or invalid', name;
    END IF;
  END LOOP;
END $$;
