-- Slack の取り込みの再参加で、返信より後に退出した人を再参加させない。
-- 取り込みの返信は作成日時が Slack の投稿時刻（過去）なので、取り込みが遅れる（429・再試行・同期の追いつき）と、
-- 返信より後に zoo で退出した人を、返信を取り込んだ時点で再参加させてしまっていた。
-- 取り込みの返信の再参加は、退出より後の返信（left_at < created_at）に限る。zoo の返信は従来どおり常に再参加する。
-- 関数の置き換えだけで、テーブル・列・制約は変えない。再Openは、返信の時刻が活動時刻を進めるので、そのままにする。
CREATE OR REPLACE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE parent_author text; live_post boolean; posted_now boolean;
BEGIN
  IF current_setting('zoo.thread_migration',true)='on' THEN RETURN NULL; END IF;
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  SELECT user_id INTO parent_author FROM messages
    WHERE id=NEW.parent_id AND deleted_at IS NULL FOR NO KEY UPDATE;
  IF NOT FOUND THEN RETURN NULL; END IF;
  posted_now:=TG_OP='INSERT' AND NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC');
  live_post:=posted_now OR (TG_OP='INSERT' AND current_setting('zoo.thread_live_import',true)='on');
  INSERT INTO thread_participants(thread_id,user_id)
    VALUES(NEW.parent_id,parent_author),(NEW.parent_id,NEW.user_id) ON CONFLICT DO NOTHING;
  IF live_post THEN
    -- 取り込みの返信は作成日時が過去なので、遅れて届くと、返信より後に退出した人を再参加させてしまう。
    -- 返信より後の退出は取り消さない（zoo の返信は posted_now なので、従来どおり常に再参加する）。
    UPDATE thread_participants SET left_at=NULL,joined_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC'
      WHERE thread_id=NEW.parent_id AND user_id=NEW.user_id AND left_at IS NOT NULL
        AND (posted_now OR left_at < NEW.created_at);
  END IF;
  UPDATE messages SET
    thread_activity_at=GREATEST(thread_activity_at,created_at,NEW.created_at),
    thread_archive_at=CASE WHEN live_post THEN NULL ELSE thread_archive_at END
    WHERE id=NEW.parent_id;
  RETURN NULL;
END $$;
