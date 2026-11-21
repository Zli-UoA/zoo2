-- atlas:txmode none

-- 全文検索のための拡張とインデックス。pg_bigm は本番では CNPG の Database リソースが先に作る
-- （Zli-UoA/zoo2-infra）。作成済みなら、スーパーユーザーでない zoo2 ロールでも IF NOT EXISTS は通る。
CREATE EXTENSION IF NOT EXISTS pg_bigm WITH SCHEMA public;
CREATE EXTENSION IF NOT EXISTS btree_gin WITH SCHEMA public;
-- 書き込みを止めないよう CONCURRENTLY で作る。
CREATE INDEX CONCURRENTLY IF NOT EXISTS "messages_search_month" ON "messages" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(content) public.gin_bigm_ops) WHERE ((deleted_at IS NULL) AND (content <> ''::text));
CREATE INDEX CONCURRENTLY IF NOT EXISTS "attachments_search_month" ON "attachments" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower(file_name) public.gin_bigm_ops) WHERE (message_id IS NOT NULL);
CREATE INDEX CONCURRENTLY IF NOT EXISTS "channel_notes_search_month" ON "channel_notes" USING gin ((((date_part('year', created_at) * 12 + date_part('month', created_at)))::integer) public.int4_ops, lower((((title)::text || E'\n') || content)) public.gin_bigm_ops) WHERE (deleted_at IS NULL);
