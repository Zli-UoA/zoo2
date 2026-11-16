-- atlas:txmode none

-- Create index "messages_channel_author_latest" to table: "messages"
CREATE INDEX CONCURRENTLY "messages_channel_author_latest" ON "messages" ("channel_id", "user_id", "created_at" DESC, "id" DESC) WHERE ((parent_id IS NULL) AND (deleted_at IS NULL));
-- Drop index "idx_slack_channel_syncs_due" from table: "slack_channel_syncs"

-- Create index "slack_channel_syncs_due_order" to table: "slack_channel_syncs"
CREATE INDEX CONCURRENTLY "slack_channel_syncs_due_order" ON "slack_channel_syncs" ("available_at", "team_id", "slack_channel_id");
-- Create index "slack_channel_wakes_due_order" to table: "slack_channel_wakes"
CREATE INDEX CONCURRENTLY "slack_channel_wakes_due_order" ON "slack_channel_wakes" ("first_at", "team_id", "slack_channel_id");
-- Create index "slack_import_jobs_active_team" to table: "slack_import_jobs"
CREATE INDEX CONCURRENTLY "slack_import_jobs_active_team" ON "slack_import_jobs" ("team_id") WHERE (status = ANY (ARRAY['analyzing'::text, 'importing'::text]));
-- Create "channel_message_authors" table
CREATE TABLE "channel_message_authors" (
  "channel_id" text NOT NULL,
  "user_id" text NOT NULL,
  "latest_at" timestamp(3) NULL,
  PRIMARY KEY ("channel_id", "user_id"),
  CONSTRAINT "channel_message_authors_channel_id_fkey" FOREIGN KEY ("channel_id") REFERENCES "channels" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT "channel_message_authors_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE CASCADE
);
-- Create index "channel_message_authors_latest" to table: "channel_message_authors"
CREATE INDEX CONCURRENTLY "channel_message_authors_latest" ON "channel_message_authors" ("channel_id", "latest_at" DESC, "user_id" DESC) WHERE (latest_at IS NOT NULL);
-- Create index "channel_message_authors_user" to table: "channel_message_authors"
CREATE INDEX CONCURRENTLY "channel_message_authors_user" ON "channel_message_authors" ("user_id", "channel_id");

CREATE FUNCTION maintain_channel_message_authors() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE before_row messages%ROWTYPE; after_row messages%ROWTYPE; item record; latest timestamp(3);
BEGIN
  IF TG_OP<>'INSERT' THEN before_row:=OLD; END IF;
  IF TG_OP<>'DELETE' THEN after_row:=NEW; END IF;
  IF TG_OP='UPDATE' AND (OLD.channel_id,OLD.user_id,OLD.created_at,OLD.parent_id,OLD.deleted_at)
    IS NOT DISTINCT FROM (NEW.channel_id,NEW.user_id,NEW.created_at,NEW.parent_id,NEW.deleted_at) THEN RETURN NULL; END IF;
  FOR item IN
    SELECT DISTINCT channel_id,user_id FROM (VALUES
      (before_row.channel_id,before_row.user_id,before_row.parent_id,before_row.deleted_at),
      (after_row.channel_id,after_row.user_id,after_row.parent_id,after_row.deleted_at)
    ) keys(channel_id,user_id,parent_id,deleted_at)
    WHERE channel_id IS NOT NULL AND parent_id IS NULL AND deleted_at IS NULL ORDER BY channel_id,user_id
  LOOP
    -- Cascading parent deletion/rename must not recreate an obsolete FK key.
    IF NOT EXISTS(SELECT 1 FROM channels WHERE id=item.channel_id) OR NOT EXISTS(SELECT 1 FROM users WHERE id=item.user_id) THEN
      DELETE FROM channel_message_authors WHERE channel_id=item.channel_id AND user_id=item.user_id;
      CONTINUE;
    END IF;
    INSERT INTO channel_message_authors(channel_id,user_id) VALUES(item.channel_id,item.user_id) ON CONFLICT DO NOTHING;
    PERFORM 1 FROM channel_message_authors WHERE channel_id=item.channel_id AND user_id=item.user_id FOR UPDATE;
    SELECT created_at INTO latest FROM messages WHERE channel_id=item.channel_id AND user_id=item.user_id
      AND parent_id IS NULL AND deleted_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1;
    UPDATE channel_message_authors SET latest_at=latest WHERE channel_id=item.channel_id AND user_id=item.user_id
      AND latest_at IS DISTINCT FROM latest;
  END LOOP;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_channel_author_latest AFTER INSERT OR DELETE OR UPDATE OF channel_id,user_id,created_at,parent_id,deleted_at ON messages
  FOR EACH ROW EXECUTE FUNCTION maintain_channel_message_authors();

-- Lock the summary before taking a fresh source snapshot. A source writer
-- cannot commit while its AFTER trigger waits for this summary key.
DO $$
DECLARE after_channel text; after_user text; batch jsonb; item record; latest timestamp(3);
BEGIN
 LOOP
  IF after_channel IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k) ORDER BY channel_id,user_id) INTO batch FROM (
    SELECT DISTINCT channel_id,user_id FROM messages WHERE parent_id IS NULL AND deleted_at IS NULL ORDER BY channel_id,user_id LIMIT 100
   ) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k) ORDER BY channel_id,user_id) INTO batch FROM (
    SELECT DISTINCT channel_id,user_id FROM messages WHERE parent_id IS NULL AND deleted_at IS NULL AND (channel_id,user_id)>(after_channel,after_user) ORDER BY channel_id,user_id LIMIT 100
   ) k;
  END IF;
  EXIT WHEN batch IS NULL;
  FOR item IN SELECT * FROM jsonb_to_recordset(batch) AS k(channel_id text,user_id text) ORDER BY channel_id,user_id LOOP
   PERFORM 1 FROM users WHERE id=item.user_id FOR KEY SHARE;
   IF NOT FOUND THEN CONTINUE; END IF;
   PERFORM 1 FROM channels WHERE id=item.channel_id FOR KEY SHARE;
   IF NOT FOUND THEN CONTINUE; END IF;
   INSERT INTO channel_message_authors(channel_id,user_id) VALUES(item.channel_id,item.user_id) ON CONFLICT DO NOTHING;
   PERFORM 1 FROM channel_message_authors WHERE channel_id=item.channel_id AND user_id=item.user_id FOR UPDATE;
   SELECT created_at INTO latest FROM messages WHERE channel_id=item.channel_id AND user_id=item.user_id AND parent_id IS NULL AND deleted_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1;
   UPDATE channel_message_authors SET latest_at=latest WHERE channel_id=item.channel_id AND user_id=item.user_id AND latest_at IS DISTINCT FROM latest;
  END LOOP;
  after_channel:=batch->-1->>'channel_id'; after_user:=batch->-1->>'user_id';
  COMMIT;
 END LOOP;
END $$;

DROP INDEX CONCURRENTLY idx_slack_channel_syncs_due;
ANALYZE channel_message_authors;
ANALYZE messages;
ANALYZE slack_import_jobs;
ANALYZE slack_channel_syncs;
ANALYZE slack_channel_wakes;
