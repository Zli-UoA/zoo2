-- Slack の履歴には、削除済みの親への返信も含まれる。スレッド化のトリガーは通常、削除済みの親への
-- 返信を拒否するが、取り込み (ZIP・差分同期・ヒント・再確認) は履歴を書くので、`zoo.slack_import` が
-- 立っている間だけ許可する。再取り込みや、後から見つかった返信も、削除された親のスレッドに残る。
CREATE OR REPLACE FUNCTION normalize_thread_parent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE root_id text;
BEGIN
  IF NEW.parent_id IS NULL THEN RETURN NEW; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NEW; END IF;
  IF (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN RETURN NEW; END IF;
  root_id:=thread_root(NEW.parent_id);
  -- Slack の履歴の取り込みは、削除済みの親への返信も書く (zoo.slack_import)。
  IF root_id IS NULL OR root_id=NEW.id OR NOT EXISTS (
    SELECT 1 FROM messages WHERE id=root_id AND channel_id=NEW.channel_id AND (
      deleted_at IS NULL
      OR current_setting('zoo.thread_migration',true)='on'
      OR current_setting('zoo.slack_import',true)='on'
    )
  ) THEN RAISE EXCEPTION 'Invalid thread parent' USING ERRCODE='23514'; END IF;
  -- The encrypted payload authenticates parent_id; never silently rewrite it.
  IF NEW.ciphertext IS NOT NULL AND (root_id IS DISTINCT FROM NEW.parent_id OR
      (TG_OP='UPDATE' AND NEW.parent_id IS DISTINCT FROM OLD.parent_id)) THEN
    RAISE EXCEPTION 'Encrypted thread parent cannot be normalized' USING ERRCODE='23514';
  END IF;
  NEW.reply_target_id:=COALESCE(NEW.reply_target_id,CASE WHEN TG_OP='UPDATE' AND current_setting('zoo.thread_migration',true)='on' THEN OLD.parent_id ELSE NEW.parent_id END);
  NEW.parent_id:=root_id;
  -- All membership changes and archival serialize on this root before reply triggers run.
  PERFORM 1 FROM messages WHERE id=root_id FOR NO KEY UPDATE;
  RETURN NEW;
END $$;
