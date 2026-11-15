
-- Thread membership survives archival; an explicit leave is never inferred from history.
CREATE TABLE thread_participants (
  thread_id text NOT NULL REFERENCES messages(id) ON DELETE CASCADE ON UPDATE CASCADE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE,
  joined_at timestamp(3) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  left_at timestamp(3),
  updated_at timestamp(3) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  PRIMARY KEY (thread_id,user_id)
);
CREATE INDEX thread_participants_joined ON thread_participants(user_id,thread_id) WHERE left_at IS NULL;
ALTER TABLE messages ADD COLUMN reply_target_id text;
ALTER TABLE home_posts ADD COLUMN reply_target_id text;

-- Rollout is explicit: deploy readers/writers before normalizing historical data.
CREATE TABLE thread_rollout (
  id boolean PRIMARY KEY DEFAULT true CHECK(id),
  phase text NOT NULL DEFAULT 'collecting' CHECK(phase IN ('collecting','normalizing','ready'))
);
INSERT INTO thread_rollout(id) VALUES(true);
CREATE TABLE thread_migration_completed (
  thread_id text PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  completed_at timestamptz NOT NULL DEFAULT now()
);

-- Keep the original reply target separately, including targets in legacy URLs.
CREATE FUNCTION thread_root(target text) RETURNS text LANGUAGE plpgsql STABLE AS $$
DECLARE current_id text:=target; next_id text; seen text[]:='{}';
BEGIN
  LOOP
    IF current_id=ANY(seen) THEN RAISE EXCEPTION 'Cyclic thread parent' USING ERRCODE='23514'; END IF;
    seen:=array_append(seen,current_id);
    SELECT parent_id INTO next_id FROM messages WHERE id=current_id;
    IF NOT FOUND THEN RETURN NULL; END IF;
    IF next_id IS NULL THEN RETURN current_id; END IF;
    current_id:=next_id;
  END LOOP;
END $$;

CREATE FUNCTION normalize_thread_parent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE root_id text;
BEGIN
  IF NEW.parent_id IS NULL THEN RETURN NEW; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NEW; END IF;
  IF (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN RETURN NEW; END IF;
  root_id:=thread_root(NEW.parent_id);
  IF root_id IS NULL OR root_id=NEW.id OR NOT EXISTS (
    SELECT 1 FROM messages WHERE id=root_id AND channel_id=NEW.channel_id AND (deleted_at IS NULL OR current_setting('zoo.thread_migration',true)='on')
  ) THEN RAISE EXCEPTION 'Invalid thread parent' USING ERRCODE='23514'; END IF;
  NEW.reply_target_id:=COALESCE(NEW.reply_target_id,NEW.parent_id);
  NEW.parent_id:=root_id;
  -- All membership changes and archival serialize on this root before reply triggers run.
  PERFORM 1 FROM messages WHERE id=root_id FOR NO KEY UPDATE;
  RETURN NEW;
END $$;
CREATE TRIGGER messages_00_thread_parent BEFORE INSERT OR UPDATE OF parent_id ON messages
  FOR EACH ROW EXECUTE FUNCTION normalize_thread_parent();

CREATE FUNCTION normalize_home_thread_parent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE current_id text:=NEW.parent_id; next_id text; seen text[]:=ARRAY[NEW.id];
BEGIN
  IF current_id IS NULL THEN RETURN NEW; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NEW; END IF;
  IF (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN RETURN NEW; END IF;
  NEW.reply_target_id:=COALESCE(NEW.reply_target_id,CASE WHEN TG_OP='UPDATE' AND current_setting('zoo.thread_migration',true)='on' THEN OLD.parent_id ELSE current_id END);
  LOOP
    IF current_id=ANY(seen) THEN RAISE EXCEPTION 'Cyclic Home reply' USING ERRCODE='23514'; END IF;
    seen:=array_append(seen,current_id);
    SELECT parent_id INTO next_id FROM home_posts WHERE id=current_id;
    IF NOT FOUND THEN SELECT parent_id INTO next_id FROM messages WHERE id=current_id; END IF;
    IF next_id IS NULL THEN EXIT; END IF;
    current_id:=next_id;
  END LOOP;
  NEW.parent_id:=current_id;
  -- parent_uri and reply_actor retain the actual federation reply target.
  RETURN NEW;
END $$;
CREATE TRIGGER home_posts_00_thread_parent BEFORE INSERT OR UPDATE OF parent_id ON home_posts
  FOR EACH ROW EXECUTE FUNCTION normalize_home_thread_parent();

CREATE OR REPLACE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE parent_author text; live_post boolean;
BEGIN
  IF current_setting('zoo.thread_migration',true)='on' THEN RETURN NULL; END IF;
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  SELECT user_id INTO parent_author FROM messages
    WHERE id=NEW.parent_id AND deleted_at IS NULL FOR NO KEY UPDATE;
  IF NOT FOUND THEN RETURN NULL; END IF;
  live_post:=TG_OP='INSERT' AND NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC');
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

CREATE OR REPLACE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record; channel_ref text; encrypted_event text;
BEGIN
  IF current_setting('zoo.thread_migration',true)='on' THEN RETURN NULL; END IF;
 channel_ref:=(CASE WHEN TG_OP='DELETE' THEN OLD.channel_id ELSE NEW.channel_id END);
 IF EXISTS(SELECT 1 FROM channels WHERE id=channel_ref AND encrypted) THEN
  row_data:=(CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END);
  old_data:=(CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END);
  encrypted_event:=(CASE
   WHEN TG_OP='INSERT' THEN 'message.created'
   WHEN TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND old_data->>'deleted_at' IS NULL THEN 'message.deleted'
   WHEN (row_data->'ciphertext',row_data->'revision',row_data->'epoch') IS DISTINCT FROM (old_data->'ciphertext',old_data->'revision',old_data->'epoch') THEN 'message.updated'
   ELSE NULL END);
  IF encrypted_event IS NULL THEN RETURN NULL; END IF;
  data:=jsonb_build_object('id',row_data->'id','channel_id',row_data->'channel_id','parent_id',row_data->'parent_id',
   'user_id',row_data->'user_id','message_kind',row_data->'message_kind','epoch',row_data->'epoch','revision',row_data->'revision',
   'ciphertext',CASE WHEN encrypted_event='message.deleted' THEN 'null'::jsonb ELSE row_data->'ciphertext' END,'encrypted',true,
   'created_at',row_data->'created_at','updated_at',row_data->'updated_at','deleted_at',row_data->'deleted_at');
  FOR installation IN SELECT i.id FROM e2ee_bot_grants g JOIN apps a ON a.bot_user_id=g.bot_user_id JOIN app_installations i ON i.app_id=a.id
   WHERE g.channel_id=channel_ref AND i.active AND a.status='installed' AND i.scopes::jsonb ? 'e2ee:device'
  LOOP
   PERFORM app_emit(encrypted_event,data,row_data->>'user_id',channel_ref,NULL,installation.id);
  END LOOP;
  RETURN NULL;
 END IF;
 row_data:=(CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END)-'display_data';
 old_data:=(CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END)-'display_data';
 data:=row_data||jsonb_build_object('previous',old_data);
 IF TG_OP='INSERT' THEN
  PERFORM app_emit('message.created',data,NEW.user_id,NEW.channel_id);
  IF NEW.parent_id IS NOT NULL THEN PERFORM app_emit('message.replied',jsonb_build_object('parent_id',NEW.parent_id,'reply',row_data),NEW.user_id,NEW.channel_id);END IF;
  IF NEW.broadcast THEN PERFORM app_emit('message.thread_broadcast',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.message_kind='emote' THEN PERFORM app_emit('message.emote',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.metadata IS NOT NULL THEN PERFORM app_emit('message.metadata_posted',jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb),NEW.user_id,NEW.channel_id);END IF;
  FOR installation IN SELECT i.*,a.bot_user_id,u.user_name FROM app_installations i JOIN apps a ON a.id=i.app_id JOIN users u ON u.id=a.bot_user_id WHERE i.active AND a.status='installed'
  LOOP
   IF is_mention(NEW.content,installation.user_name) THEN PERFORM app_emit('app.mentioned',data,NEW.user_id,NEW.channel_id,NULL,installation.id);END IF;
   IF EXISTS(SELECT 1 FROM jsonb_array_elements_text(COALESCE(installation.manifest::jsonb->'link_domains','[]')) domain WHERE NEW.content ~ ('https://' || replace(domain.value,'.','[.]') || '([/:?#[:space:]]|$)')) THEN
    PERFORM app_emit('link.shared',data,NEW.user_id,NEW.channel_id,NULL,installation.id);
   END IF;
  END LOOP;
 ELSIF TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND old_data->>'deleted_at' IS NULL THEN
  PERFORM app_emit('message.deleted',data,row_data->>'user_id',row_data->>'channel_id');
  IF old_data->>'metadata' IS NOT NULL THEN PERFORM app_emit('message.metadata_deleted',jsonb_build_object('id',OLD.id,'previous_metadata',OLD.metadata::jsonb),OLD.user_id,OLD.channel_id);END IF;
 ELSIF (row_data-'thread_archive_at'-'thread_activity_at') IS DISTINCT FROM (old_data-'thread_archive_at'-'thread_activity_at') THEN
  PERFORM app_emit('message.updated',data,NEW.user_id,NEW.channel_id);
  IF NEW.metadata IS DISTINCT FROM OLD.metadata THEN
   PERFORM app_emit(CASE WHEN NEW.metadata IS NULL THEN 'message.metadata_deleted' WHEN OLD.metadata IS NULL THEN 'message.metadata_posted' ELSE 'message.metadata_updated' END,
    jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb,'previous_metadata',OLD.metadata::jsonb),NEW.user_id,NEW.channel_id);
  END IF;
 END IF;
 RETURN NULL;
END $$;

CREATE OR REPLACE FUNCTION sync_message_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF current_setting('zoo.thread_migration',true)='on' THEN RETURN NULL; END IF;
  -- Include the row and both roots in the same deterministic lock order.
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.parent_id END
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.parent_id END
  ) targets WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;

-- One invocation is one short transaction. Never lock all messages to migrate.
CREATE FUNCTION migrate_thread_batch(batch_size integer DEFAULT 1) RETURNS integer LANGUAGE plpgsql AS $$
DECLARE root record; changed integer:=0; targets text[]; target text;
BEGIN
  IF (SELECT phase FROM thread_rollout WHERE id)<>'normalizing' THEN
    RAISE EXCEPTION 'Start thread normalization after deploying compatible clients and backend';
  END IF;
  PERFORM set_config('zoo.thread_migration','on',true);
  FOR root IN SELECT m.id FROM messages m
    WHERE m.parent_id IS NULL AND (m.thread_activity_at IS NOT NULL OR EXISTS(SELECT 1 FROM messages child WHERE child.parent_id=m.id))
      AND NOT EXISTS(SELECT 1 FROM thread_migration_completed done WHERE done.thread_id=m.id)
    ORDER BY m.id LIMIT greatest(1,least(batch_size,100)) FOR NO KEY UPDATE SKIP LOCKED
  LOOP
    WITH RECURSIVE descendants AS (
      SELECT id FROM messages WHERE id=root.id
      UNION ALL SELECT child.id FROM messages child JOIN descendants d ON child.parent_id=d.id
    ) SELECT array_agg(id) INTO targets FROM descendants;
    -- Preserve every user's explicit root decision; merge nested membership only
    -- when the root has no decision yet. A nested leave takes precedence.
    INSERT INTO thread_participants(thread_id,user_id,joined_at,left_at)
      SELECT root.id,user_id,min(joined_at),max(left_at) FROM thread_participants
      WHERE thread_id=ANY(targets) GROUP BY user_id ON CONFLICT DO NOTHING;
    INSERT INTO thread_participants(thread_id,user_id)
      SELECT DISTINCT root.id,user_id FROM messages WHERE id=ANY(targets) AND deleted_at IS NULL
      ON CONFLICT DO NOTHING;
    UPDATE messages m SET thread_activity_at=states.activity_at,thread_archive_at=states.archive_at
      FROM (SELECT max(thread_activity_at) AS activity_at,
        CASE WHEN bool_or(thread_archive_at IS NULL) THEN NULL ELSE max(thread_archive_at) END AS archive_at
        FROM messages WHERE id=ANY(targets) AND thread_activity_at IS NOT NULL) states
      WHERE m.id=root.id;
    UPDATE messages SET reply_target_id=COALESCE(reply_target_id,parent_id),parent_id=root.id,
      thread_activity_at=NULL,thread_archive_at=NULL
      WHERE id=ANY(targets) AND id<>root.id;
    -- Rebuild only the migrated conversation, not the whole message table.
    FOREACH target IN ARRAY targets LOOP PERFORM refresh_message_display(target); END LOOP;
    INSERT INTO thread_migration_completed(thread_id) VALUES(root.id) ON CONFLICT DO NOTHING;
    changed:=changed+1;
  END LOOP;
  PERFORM set_config('zoo.thread_migration','off',true);
  RETURN changed;
END $$;

CREATE FUNCTION home_thread_root(target text) RETURNS text LANGUAGE plpgsql STABLE AS $$
DECLARE current_id text:=target; next_id text; seen text[]:='{}';
BEGIN
  LOOP
    IF current_id=ANY(seen) THEN RAISE EXCEPTION 'Cyclic Home parent' USING ERRCODE='23514'; END IF;
    seen:=array_append(seen,current_id);
    SELECT parent_id INTO next_id FROM home_posts WHERE id=current_id;
    IF NOT FOUND THEN SELECT parent_id INTO next_id FROM messages WHERE id=current_id; END IF;
    IF next_id IS NULL THEN RETURN current_id; END IF;
    current_id:=next_id;
  END LOOP;
END $$;

CREATE FUNCTION migrate_home_thread_batch(batch_size integer DEFAULT 100) RETURNS integer LANGUAGE plpgsql AS $$
DECLARE affected integer;
BEGIN
  IF (SELECT phase FROM thread_rollout WHERE id)<>'normalizing' THEN RAISE EXCEPTION 'Thread rollout is not normalizing'; END IF;
  PERFORM set_config('zoo.thread_migration','on',true);
  WITH batch AS (
    SELECT h.id,home_thread_root(h.parent_id) AS root_id FROM home_posts h
      WHERE h.parent_id IS NOT NULL AND h.parent_id IS DISTINCT FROM home_thread_root(h.parent_id)
      ORDER BY h.id LIMIT greatest(1,least(batch_size,1000)) FOR UPDATE SKIP LOCKED
  ) UPDATE home_posts h SET reply_target_id=COALESCE(h.reply_target_id,h.parent_id),parent_id=b.root_id
    FROM batch b WHERE h.id=b.id;
  GET DIAGNOSTICS affected=ROW_COUNT;
  PERFORM set_config('zoo.thread_migration','off',true);
  RETURN affected;
END $$;
