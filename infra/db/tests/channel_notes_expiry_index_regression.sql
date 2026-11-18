\set ON_ERROR_STOP on

-- Run against a database after applying the canonical migration history.
-- This asserts the column order that the former IF NOT EXISTS migration failed
-- to repair when an older (deleted_at, id) index already had this name.
DO $$
DECLARE
  index_oid oid;
  key_count smallint;
  keys text[];
  predicate text;
  index_valid boolean;
BEGIN
  SELECT indexrelid, indnkeyatts, pg_get_expr(indpred, indrelid), indisvalid
    INTO index_oid, key_count, predicate, index_valid
  FROM pg_index
  WHERE indexrelid = to_regclass('public.idx_channel_notes_expired');

  IF index_oid IS NULL THEN
    RAISE EXCEPTION 'idx_channel_notes_expired is missing';
  END IF;

  SELECT array_agg(pg_get_indexdef(index_oid, position, true) ORDER BY position)
    INTO keys
  FROM generate_series(1, key_count) AS positions(position);

  IF NOT COALESCE(index_valid, false) THEN
    RAISE EXCEPTION 'idx_channel_notes_expired is invalid; drop it with DROP INDEX CONCURRENTLY and retry Atlas';
  END IF;
  IF keys IS DISTINCT FROM ARRAY['channel_id', 'deleted_at', 'id']::text[] THEN
    RAISE EXCEPTION 'wrong idx_channel_notes_expired key order: %', keys;
  END IF;
  IF predicate IS DISTINCT FROM '(deleted_at IS NOT NULL)' THEN
    RAISE EXCEPTION 'wrong idx_channel_notes_expired predicate: %', predicate;
  END IF;
END $$;
