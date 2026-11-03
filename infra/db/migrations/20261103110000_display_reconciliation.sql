-- Reconcile both sides after commit. A source row can be invisible when a
-- metadata transaction scans it, regardless of which transaction commits first.
-- Transaction generations keep a later writer's job independent of an older
-- job being consumed, without cross-transaction conflicts on queue insertion.
ALTER TABLE display_refresh_queue DROP CONSTRAINT display_refresh_queue_pkey;
ALTER TABLE display_refresh_queue DROP CONSTRAINT display_refresh_queue_kind_check;
ALTER TABLE display_refresh_queue ADD COLUMN generation bigint NOT NULL DEFAULT txid_current();
ALTER TABLE display_refresh_queue ADD COLUMN cursor_id text NOT NULL DEFAULT '';
ALTER TABLE display_refresh_queue ADD PRIMARY KEY(kind,target,generation);
ALTER TABLE display_refresh_queue ADD CHECK(kind IN (
  'message','notification','user_messages','user_notifications','emoji','emoji_image','actor','channel'
));

-- Do not scan every row-repaint job when the scope queue is empty.
CREATE INDEX display_refresh_scopes_idx ON display_refresh_queue(kind,target,generation)
  WHERE kind NOT IN ('message','notification');

CREATE OR REPLACE FUNCTION sync_user_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.avatar_url,OLD.is_bot,OLD.app_id)
    IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.avatar_url,NEW.is_bot,NEW.app_id) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES
    ('user_messages',NEW.id),('user_notifications',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;

CREATE FUNCTION enqueue_emoji_display_scopes(changes jsonb) RETURNS void LANGUAGE sql AS $$
  INSERT INTO display_refresh_queue(kind,target)
    SELECT 'emoji',e->>'name' FROM jsonb_array_elements(changes) e
    UNION SELECT 'emoji',jsonb_array_elements_text(e->'aliases') FROM jsonb_array_elements(changes) e
    UNION SELECT 'emoji_image',e->>'image_id' FROM jsonb_array_elements(changes) e
      WHERE COALESCE(e->>'image_id','')<>''
    ON CONFLICT DO NOTHING;
$$;
CREATE OR REPLACE FUNCTION sync_emoji_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((
    WITH changed AS (
      SELECT name,url,aliases,width,height,image_id FROM new_emojis
      EXCEPT SELECT name,url,aliases,width,height,image_id FROM old_emojis
    ), removed AS (
      SELECT name,url,aliases,width,height,image_id FROM old_emojis
      EXCEPT SELECT name,url,aliases,width,height,image_id FROM new_emojis
    ), affected AS (SELECT * FROM changed UNION SELECT * FROM removed)
    SELECT jsonb_agg(to_jsonb(a)) FROM affected a
  ),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION sync_emoji_insert_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM new_emojis e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION sync_emoji_delete_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM old_emojis e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION sync_federated_actor_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.blocked) IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.blocked) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('actor',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF OLD IS NOT DISTINCT FROM NEW THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('channel',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;

-- This SQL function is inlined so the caller's cursor and LIMIT are pushed
-- into the lookup. Each scope returns only one row kind, sharing one ID cursor.
CREATE FUNCTION display_scope_targets(scope_kind text,scope_target text)
RETURNS TABLE(row_kind text,row_id text) LANGUAGE sql STABLE AS $$
  SELECT 'message'::text,m.id FROM messages m WHERE
    (scope_kind='user_messages' AND (m.user_id=scope_target OR EXISTS (
      SELECT 1 FROM message_reactions r WHERE r.message_id=m.id AND r.user_id=scope_target))) OR
    (scope_kind='emoji' AND (strpos(m.content,':'||scope_target||':')>0 OR EXISTS (
      SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND r.emoji=':'||scope_target||':'))) OR
    (scope_kind='emoji_image' AND EXISTS (
      SELECT 1 FROM apub_reactions r WHERE r.post_id=m.id AND r.emoji->>'image_id'=scope_target)) OR
    (scope_kind='actor' AND EXISTS (
      SELECT 1 FROM apub_reactions r WHERE r.post_id=m.id AND r.actor_id=scope_target))
  UNION ALL
  SELECT 'notification',n.id FROM notification_events n WHERE n.source='channel' AND (
    (scope_kind='user_notifications' AND n.actor_id=scope_target) OR
    (scope_kind='channel' AND n.channel_id=scope_target));
$$;
CREATE FUNCTION expand_next_display_scope() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
DECLARE job record; item record; last_id text; processed integer := 0;
BEGIN
  SELECT * INTO job FROM display_refresh_queue
    WHERE kind NOT IN ('message','notification') ORDER BY kind,target,generation LIMIT 1 FOR UPDATE SKIP LOCKED;
  IF NOT FOUND THEN RETURN false; END IF;
  FOR item IN SELECT * FROM display_scope_targets(job.kind,job.target)
    WHERE row_id>job.cursor_id ORDER BY row_id LIMIT 128 LOOP
    INSERT INTO display_refresh_queue(kind,target) VALUES(item.row_kind,item.row_id) ON CONFLICT DO NOTHING;
    last_id := item.row_id;
    processed := processed+1;
  END LOOP;
  IF processed<128 THEN
    DELETE FROM display_refresh_queue WHERE kind=job.kind AND target=job.target AND generation=job.generation;
  ELSE
    UPDATE display_refresh_queue SET cursor_id=last_id WHERE kind=job.kind AND target=job.target AND generation=job.generation;
  END IF;
  RETURN true;
END;
$$;

CREATE OR REPLACE FUNCTION sync_message_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
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

CREATE OR REPLACE FUNCTION sync_message_relation_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.message_id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.message_id END
  ) ids WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION sync_federated_reaction_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF TG_OP <> 'DELETE' AND jsonb_typeof(NEW.emoji)='object' THEN
    INSERT INTO apub_emoji_images(id,url) VALUES (NEW.emoji->>'image_id',NEW.emoji->>'url')
      ON CONFLICT(id) DO NOTHING;
  END IF;
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.post_id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.post_id END
  ) ids WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION sync_notification_display() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM refresh_notification_display(NEW.id);
  IF NEW.source='channel' THEN
    INSERT INTO display_refresh_queue(kind,target) VALUES ('notification',NEW.id) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION sync_message_notifications_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF current_setting('zoo.display_refresh',true)='metadata' THEN
    INSERT INTO display_refresh_queue(kind,target) SELECT 'notification',id FROM notification_events
      WHERE source='channel' AND message_id=NEW.id ORDER BY id ON CONFLICT DO NOTHING;
    RETURN NULL;
  END IF;
  FOR target IN SELECT n.id FROM notification_events n
    WHERE n.source='channel' AND n.message_id=NEW.id AND (
      OLD.content IS DISTINCT FROM NEW.content OR OLD.user_id IS DISTINCT FROM NEW.user_id OR
      OLD.deleted_at IS DISTINCT FROM NEW.deleted_at OR
      (OLD.display_data-'reactions') IS DISTINCT FROM (NEW.display_data-'reactions') OR
      n.notification_type='reaction' AND n.actor_id IN (
        SELECT reaction->>'user_id' FROM (
          (SELECT value AS reaction FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]')))
          UNION
          (SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]')))
        ) changed
      )
    ) ORDER BY n.id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION refresh_next_display() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
DECLARE job record;
BEGIN
  FOR job IN SELECT * FROM display_refresh_queue WHERE kind IN ('message','notification') ORDER BY kind,target,generation LIMIT 32 FOR UPDATE SKIP LOCKED LOOP
    IF job.kind='message' THEN
      IF EXISTS(SELECT 1 FROM messages WHERE id=job.target) THEN
        PERFORM 1 FROM messages WHERE id=job.target FOR NO KEY UPDATE SKIP LOCKED;
        IF NOT FOUND THEN CONTINUE; END IF;
        PERFORM set_config('zoo.display_refresh','metadata',true);
        PERFORM refresh_message_display(job.target);
        PERFORM set_config('zoo.display_refresh','',true);
      END IF;
    ELSE
      IF EXISTS(SELECT 1 FROM notification_events WHERE id=job.target) THEN
        PERFORM 1 FROM notification_events WHERE id=job.target FOR NO KEY UPDATE SKIP LOCKED;
        IF NOT FOUND THEN CONTINUE; END IF;
        PERFORM refresh_notification_display(job.target);
      END IF;
    END IF;
    DELETE FROM display_refresh_queue WHERE kind=job.kind AND target=job.target AND generation=job.generation;
    RETURN true;
  END LOOP;
  RETURN false;
END;
$$;

-- Repair any pre-existing projection drift without locking source rows.
INSERT INTO display_refresh_queue(kind,target) SELECT 'message',id FROM messages ORDER BY id ON CONFLICT DO NOTHING;
INSERT INTO display_refresh_queue(kind,target) SELECT 'notification',id FROM notification_events WHERE source='channel' ORDER BY id ON CONFLICT DO NOTHING;
