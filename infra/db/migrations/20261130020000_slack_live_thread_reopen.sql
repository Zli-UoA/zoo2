-- Slack の取り込み（差分同期・イベント）が、直近の返信を、zoo の返信と同じライブの投稿として扱えるようにする。
-- 取り込みの返信は作成日時が Slack の投稿時刻なので、トランザクション開始時刻との比べだけでは、ライブと判定されない。
-- `zoo.thread_live_import` が `on` のトランザクションの INSERT は、ライブの返信として、スレッドを再Openし、
-- 退出済みの投稿者を再参加させる（Home の返信と同じ）。フラグは、バックエンドが直近の返信にだけ立てる。
-- 履歴の取り込みや、同じ投稿IDの再送（AFTER INSERT が動かない）では、立てても何も変わらない。
-- 関数の置き換えだけで、旧バックエンドはフラグを立てないので、動作は変わらない。
CREATE OR REPLACE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE parent_author text; live_post boolean;
BEGIN
  IF current_setting('zoo.thread_migration',true)='on' THEN RETURN NULL; END IF;
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  SELECT user_id INTO parent_author FROM messages
    WHERE id=NEW.parent_id AND deleted_at IS NULL FOR NO KEY UPDATE;
  IF NOT FOUND THEN RETURN NULL; END IF;
  live_post:=TG_OP='INSERT' AND (NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC')
    OR current_setting('zoo.thread_live_import',true)='on');
  INSERT INTO thread_participants(thread_id,user_id)
    VALUES(NEW.parent_id,parent_author),(NEW.parent_id,NEW.user_id) ON CONFLICT DO NOTHING;
  IF live_post THEN
    UPDATE thread_participants SET left_at=NULL,joined_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC'
      WHERE thread_id=NEW.parent_id AND user_id=NEW.user_id AND left_at IS NOT NULL;
  END IF;
  UPDATE messages SET
    thread_activity_at=GREATEST(thread_activity_at,created_at,NEW.created_at),
    thread_archive_at=CASE WHEN live_post THEN NULL ELSE thread_archive_at END
    WHERE id=NEW.parent_id;
  RETURN NULL;
END $$;
