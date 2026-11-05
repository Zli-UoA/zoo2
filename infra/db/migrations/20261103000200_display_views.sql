SET LOCAL jit=off;
ALTER TABLE workspace_emojis ADD COLUMN width integer, ADD COLUMN height integer, ADD COLUMN image_id text;
-- Canonical read models. Electric 1.x cannot replicate a JOIN VIEW, so their
-- results are cached on the source rows in the same database transaction.
ALTER TABLE messages ADD COLUMN display_data jsonb NOT NULL DEFAULT '{}';
ALTER TABLE notification_events ADD COLUMN display_data jsonb NOT NULL DEFAULT '{}';

-- Metadata changes enqueue read-model work; they never lock message rows.
-- Keep this private table out of Electric Shapes and application events.
CREATE TABLE display_refresh_queue (
  kind text NOT NULL CHECK (kind IN ('message','notification')),
  target text NOT NULL,
  PRIMARY KEY(kind,target)
);


-- Cache writes are not user edits and must not enter App event payloads.
CREATE OR REPLACE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record;
BEGIN
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
 ELSIF row_data IS DISTINCT FROM old_data THEN
  PERFORM app_emit('message.updated',data,NEW.user_id,NEW.channel_id);
  IF NEW.metadata IS DISTINCT FROM OLD.metadata THEN
   PERFORM app_emit(CASE WHEN NEW.metadata IS NULL THEN 'message.metadata_deleted' WHEN OLD.metadata IS NULL THEN 'message.metadata_posted' ELSE 'message.metadata_updated' END,
    jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb,'previous_metadata',OLD.metadata::jsonb),NEW.user_id,NEW.channel_id);
  END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE VIEW message_display_reactions AS
SELECT r.id,r.message_id,r.user_id,r.emoji,r.created_at,u.user_name,u.display_name,NULL::jsonb AS custom_emoji
FROM message_reactions r JOIN users u ON u.id=r.user_id
UNION ALL
SELECT r.activity_id,p.id,r.actor_id,
  CASE WHEN jsonb_typeof(r.emoji)='object' THEN COALESCE(
    (SELECT ':'||e.name||':' FROM workspace_emojis e WHERE e.image_id=r.emoji->>'image_id' ORDER BY e.name LIMIT 1),
    'apub:'||(r.emoji->>'image_id')||':'||(r.emoji->>'name')) ELSE r.reaction END,
  r.created_at AT TIME ZONE 'UTC',a.user_name||'@'||split_part(a.id,'/',3),a.display_name,r.emoji
FROM apub_reactions r JOIN home_posts p ON p.id=r.post_id JOIN messages m ON m.id=p.id
JOIN apub_actors a ON a.id=r.actor_id
WHERE p.channel_id IS NOT NULL AND p.deleted_at IS NULL AND m.deleted_at IS NULL
  AND r.user_id IS NULL AND NOT a.blocked;

CREATE FUNCTION message_content_emojis(content text) RETURNS jsonb LANGUAGE sql STABLE AS $$
  SELECT COALESCE(jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id)), '{}'::jsonb)
  FROM workspace_emojis e CROSS JOIN LATERAL (
    SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
  ) names WHERE strpos(content,':'||names.name||':')>0;
$$;

CREATE VIEW message_display AS
SELECT m.id, jsonb_build_object(
  'author', (SELECT jsonb_build_object('id',u.id,'user_name',u.user_name,
    'display_name',u.display_name,'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id)
    FROM users u WHERE u.id=m.user_id),
  'emojis', COALESCE((SELECT jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id))
    FROM workspace_emojis e CROSS JOIN LATERAL (
      SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
    ) names WHERE strpos(m.content,':'||names.name||':')>0 OR EXISTS (
      SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND r.emoji=':'||names.name||':'
    )), '{}'::jsonb) || COALESCE((SELECT jsonb_object_agg(r.emoji,r.custom_emoji) FROM message_display_reactions r WHERE r.message_id=m.id AND r.custom_emoji IS NOT NULL AND r.emoji LIKE 'apub:%'),'{}'::jsonb),
  'attachments', COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.created_at,a.id)
    FROM attachments a WHERE a.message_id=m.id),'[]'::jsonb),
  'reactions', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.created_at,r.id)
    FROM message_display_reactions r WHERE r.message_id=m.id),'[]'::jsonb),
  'reply_count', (SELECT count(*) FROM messages reply WHERE reply.parent_id=m.id AND reply.deleted_at IS NULL)
) AS display_data FROM messages m;

CREATE VIEW notification_display AS
SELECT n.id, jsonb_build_object(
  'message', CASE WHEN m.deleted_at IS NULL AND m.id IS NOT NULL THEN
    jsonb_build_object('id',m.id,'user_id',m.user_id,'content',left(m.content,281),
      'emojis',COALESCE((SELECT jsonb_object_agg(e.key,e.value)
        FROM jsonb_each(COALESCE(m.display_data->'emojis','{}'::jsonb)) e
        WHERE strpos(left(m.content,281),':'||e.key||':')>0 OR
          badge.reaction->>'emoji' IN (e.key,':'||e.key||':')), '{}'::jsonb)) ELSE NULL END,
  'reactions', CASE WHEN n.notification_type='reaction' AND m.deleted_at IS NULL AND badge.reaction IS NOT NULL
    THEN jsonb_build_array(badge.reaction) ELSE '[]'::jsonb END,
  'actor', jsonb_build_object('id',u.id,'user_name',u.user_name,'display_name',u.display_name,
    'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id),
  'channel', to_jsonb(c)
) AS display_data
FROM notification_events n JOIN users u ON u.id=n.actor_id JOIN channels c ON c.id=n.channel_id
LEFT JOIN messages m ON m.id=n.message_id AND m.channel_id=n.channel_id
LEFT JOIN LATERAL (
  SELECT entry.value AS reaction FROM jsonb_array_elements(COALESCE(m.display_data->'reactions','[]'::jsonb)) AS entry(value)
  WHERE n.notification_type='reaction' AND entry.value->>'user_id'=n.actor_id
  ORDER BY entry.value->>'created_at' DESC,entry.value->>'id' LIMIT 1
) badge ON true
WHERE n.source='channel';

CREATE FUNCTION refresh_message_display(target text) RETURNS void LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  -- Serialize writers for this read model before taking the aggregate snapshot.
  PERFORM 1 FROM messages WHERE id=target FOR NO KEY UPDATE;
  UPDATE messages m SET display_data=v.display_data FROM message_display v
    WHERE m.id=target AND v.id=m.id AND m.display_data IS DISTINCT FROM v.display_data;
END;
$$;

CREATE FUNCTION refresh_notification_display(target text) RETURNS void LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM 1 FROM notification_events WHERE id=target FOR NO KEY UPDATE;
  UPDATE notification_events n SET display_data=v.display_data FROM notification_display v
    WHERE n.id=target AND v.id=n.id AND n.display_data IS DISTINCT FROM v.display_data;
END;
$$;

CREATE FUNCTION sync_message_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  -- Include the row and both roots in the same deterministic lock order.
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.parent_id END
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.parent_id END
  ) targets WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER messages_display_insert AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();
CREATE TRIGGER messages_display_update AFTER UPDATE OF content,user_id,parent_id,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();
CREATE TRIGGER messages_display_delete AFTER DELETE ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();

CREATE FUNCTION sync_message_relation_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.message_id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.message_id END
  ) ids WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER attachments_display AFTER INSERT OR UPDATE OR DELETE ON attachments FOR EACH ROW EXECUTE FUNCTION sync_message_relation_display();
CREATE TRIGGER reactions_display AFTER INSERT OR UPDATE OR DELETE ON message_reactions FOR EACH ROW EXECUTE FUNCTION sync_message_relation_display();

CREATE FUNCTION sync_notification_display() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM refresh_notification_display(NEW.id);
  RETURN NULL;
END;
$$;
CREATE TRIGGER notifications_display_insert AFTER INSERT ON notification_events FOR EACH ROW EXECUTE FUNCTION sync_notification_display();
CREATE TRIGGER notifications_display_update AFTER UPDATE OF actor_id,channel_id,message_id,notification_type,source ON notification_events FOR EACH ROW EXECUTE FUNCTION sync_notification_display();

CREATE FUNCTION sync_message_notifications_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF current_setting('zoo.display_refresh',true)='metadata' THEN
    INSERT INTO display_refresh_queue SELECT 'notification',id FROM notification_events
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
CREATE TRIGGER message_notifications_display AFTER UPDATE OF content,user_id,display_data,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_notifications_display();

CREATE FUNCTION sync_user_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.avatar_url,OLD.is_bot,OLD.app_id)
    IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.avatar_url,NEW.is_bot,NEW.app_id) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue SELECT 'message',id FROM messages WHERE user_id=NEW.id OR id IN (
    SELECT message_id FROM message_reactions WHERE user_id=NEW.id
  ) ORDER BY id ON CONFLICT DO NOTHING;
  INSERT INTO display_refresh_queue SELECT 'notification',id FROM notification_events
    WHERE source='channel' AND actor_id=NEW.id ORDER BY id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE TRIGGER users_display AFTER UPDATE OF user_name,display_name,avatar_url,is_bot,app_id ON users FOR EACH ROW EXECUTE FUNCTION sync_user_display();


CREATE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF OLD IS NOT DISTINCT FROM NEW THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue SELECT 'notification',id FROM notification_events
    WHERE source='channel' AND channel_id=NEW.id ORDER BY id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE TRIGGER channels_notification_display AFTER UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION sync_channel_notification_display();

CREATE FUNCTION sync_federated_reaction_display() RETURNS trigger LANGUAGE plpgsql AS $$
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
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER federated_reactions_display AFTER INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH ROW EXECUTE FUNCTION sync_federated_reaction_display();

CREATE FUNCTION sync_federated_actor_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.blocked) IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.blocked) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue SELECT DISTINCT 'message',post_id FROM apub_reactions
    WHERE actor_id=NEW.id ORDER BY post_id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE TRIGGER federated_actors_display AFTER UPDATE OF user_name,display_name,blocked ON apub_actors FOR EACH ROW EXECUTE FUNCTION sync_federated_actor_display();

-- Backfill while the migration holds the schema lock; old application versions
-- keep writing normally and triggers maintain the projections for them too.
CREATE FUNCTION sync_emoji_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  -- Transition tables include deleted names, aliases and old image identities.
  -- No-op updates (including startup repeats) enqueue nothing.
  INSERT INTO display_refresh_queue
  WITH changed AS (
    SELECT name,url,aliases,width,height,image_id FROM new_emojis
    EXCEPT SELECT name,url,aliases,width,height,image_id FROM old_emojis
  ), removed AS (
    SELECT name,url,aliases,width,height,image_id FROM old_emojis
    EXCEPT SELECT name,url,aliases,width,height,image_id FROM new_emojis
  ), affected AS (SELECT * FROM changed UNION SELECT * FROM removed), names AS (
    SELECT name FROM affected UNION SELECT jsonb_array_elements_text(aliases) FROM affected
  )
  SELECT 'message',m.id FROM messages m WHERE
    EXISTS(SELECT 1 FROM names n WHERE strpos(m.content,':'||n.name||':')>0) OR
    EXISTS(SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND (
      r.emoji IN (SELECT ':'||name||':' FROM names) OR
      r.emoji LIKE 'apub:%' AND split_part(r.emoji,':',2) IN (SELECT image_id FROM affected)
    )) ORDER BY m.id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
-- A common transition-table shape for all three operations keeps filtering in
-- one function while preserving both sides of renames/deletions.
CREATE FUNCTION sync_emoji_insert_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  INSERT INTO display_refresh_queue
    SELECT 'message',m.id FROM messages m WHERE EXISTS (
      SELECT 1 FROM new_emojis e WHERE strpos(m.content,':'||e.name||':')>0
        OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(e.aliases) a(name) WHERE strpos(m.content,':'||a.name||':')>0)
        OR EXISTS(SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND
          (r.emoji=':'||e.name||':' OR r.emoji IN (SELECT ':'||a||':' FROM jsonb_array_elements_text(e.aliases) a)
          OR r.emoji LIKE 'apub:%' AND split_part(r.emoji,':',2)=e.image_id))
    ) ORDER BY m.id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE FUNCTION sync_emoji_delete_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  INSERT INTO display_refresh_queue
    SELECT 'message',m.id FROM messages m WHERE EXISTS (
      SELECT 1 FROM old_emojis e WHERE strpos(m.content,':'||e.name||':')>0
        OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(e.aliases) a(name) WHERE strpos(m.content,':'||a.name||':')>0)
        OR EXISTS(SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND
          (r.emoji=':'||e.name||':' OR r.emoji IN (SELECT ':'||a||':' FROM jsonb_array_elements_text(e.aliases) a)
          OR r.emoji LIKE 'apub:%' AND split_part(r.emoji,':',2)=e.image_id))
    ) ORDER BY m.id ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE TRIGGER emojis_display_update AFTER UPDATE ON workspace_emojis REFERENCING OLD TABLE AS old_emojis NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_display();
CREATE TRIGGER emojis_display_insert AFTER INSERT ON workspace_emojis REFERENCING NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_insert_display();
CREATE TRIGGER emojis_display_delete AFTER DELETE ON workspace_emojis REFERENCING OLD TABLE AS old_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_delete_display();


-- One item per transaction: no workspace-wide gate and no multi-message lock
-- order to invert with old backends. Busy rows are retried on a later pass.
CREATE FUNCTION refresh_next_display() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
DECLARE job record;
BEGIN
  FOR job IN SELECT * FROM display_refresh_queue ORDER BY kind,target LIMIT 32 FOR UPDATE SKIP LOCKED LOOP
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
    DELETE FROM display_refresh_queue WHERE kind=job.kind AND target=job.target;
    RETURN true;
  END LOOP;
  RETURN false;
END;
$$;

INSERT INTO apub_emoji_images(id,url)
  SELECT DISTINCT emoji->>'image_id',emoji->>'url' FROM apub_reactions
  WHERE jsonb_typeof(emoji)='object' AND emoji->>'image_id' IS NOT NULL AND emoji->>'url' IS NOT NULL
  ON CONFLICT(id) DO NOTHING;
UPDATE messages m SET display_data=v.display_data FROM message_display v WHERE v.id=m.id;
UPDATE notification_events n SET display_data=v.display_data FROM notification_display v WHERE v.id=n.id;
