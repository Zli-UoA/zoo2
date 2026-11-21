-- atlas:txmode none

-- 20261130000000_search_bigm.sql の索引を確かめ直す（docs/search.md）。
-- CONCURRENTLY が途中で失敗すると INVALID の索引が残り、IF NOT EXISTS は名前だけを見て飛ばしてしまう。
-- txmode none の Atlas は、再実行すると失敗した文から続ける（成功した前の文は流さない）ので、前のファイルの
-- 作成をやり直しても INVALID のまま通りうる。ここで INVALID を消して作り直し、最後に有効かを確かめる。
--   1. 前のファイルで INVALID が残った → 下の DO が消す → 作成で作る。
--   2. ここでの作成も失敗したら、最後の DO が止める（INVALID のまま黙って通さない）。このときは、再開しても
--      失敗した作成の文から続くので、消すブロックは流れず、同じ所で止まり続ける。docs/search.md の
--      「索引の作成に失敗したとき」の手順で、INVALID の索引を消してから適用し直す。
-- 正常なら、どの文も何もしない。DROP は lock_timeout を付けて、表のロックを待ちすぎない。
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
  FOREACH name IN ARRAY ARRAY['messages_search_month', 'attachments_search_month', 'channel_notes_search_month', 'messages_search_reply_floor'] LOOP
    IF NOT EXISTS (SELECT 1 FROM pg_index WHERE indexrelid = to_regclass(name) AND indisvalid) THEN
      RAISE EXCEPTION 'search index % is missing or invalid', name;
    END IF;
  END LOOP;
END $$;
