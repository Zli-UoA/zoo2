CREATE OR REPLACE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  UPDATE messages parent SET
    thread_activity_at=GREATEST(parent.thread_activity_at,parent.created_at,NEW.created_at),
    -- INSERT 前にアーカイブされた通常の返信も再開し、過去の投稿の取り込みと区別する。
    thread_archive_at=CASE WHEN NEW.created_at >= parent.thread_archive_at
      OR NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC')
      THEN NULL ELSE parent.thread_archive_at END
    WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL;
  RETURN NULL;
END $$;

-- 既存の返信を一度だけ集計し、一覧とワーカーでは保存済みの活動時刻を使う。
UPDATE messages root SET thread_activity_at=GREATEST(root.thread_activity_at,root.created_at,replies.activity_at)
FROM (SELECT parent_id,max(created_at) AS activity_at FROM messages
      WHERE parent_id IS NOT NULL AND deleted_at IS NULL GROUP BY parent_id) replies
WHERE root.id=replies.parent_id AND root.deleted_at IS NULL
  AND root.thread_activity_at IS DISTINCT FROM GREATEST(root.thread_activity_at,root.created_at,replies.activity_at);

-- Create index "idx_messages_thread_channel_active" to table: "messages"
CREATE INDEX "idx_messages_thread_channel_active" ON "messages" ("channel_id", "thread_activity_at" DESC, "id" DESC) WHERE ((deleted_at IS NULL) AND (thread_archive_at IS NULL) AND (thread_activity_at IS NOT NULL));
-- Create index "idx_messages_thread_channel_archived" to table: "messages"
CREATE INDEX "idx_messages_thread_channel_archived" ON "messages" ("channel_id", "thread_activity_at" DESC, "id" DESC) WHERE ((deleted_at IS NULL) AND (thread_archive_at IS NOT NULL) AND (thread_activity_at IS NOT NULL));
-- Create index "idx_messages_thread_idle" to table: "messages"
CREATE INDEX "idx_messages_thread_idle" ON "messages" ("thread_activity_at", "id") WHERE ((deleted_at IS NULL) AND (thread_archive_at IS NULL) AND (thread_activity_at IS NOT NULL));
