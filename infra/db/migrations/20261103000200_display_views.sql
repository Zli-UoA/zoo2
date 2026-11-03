SET LOCAL jit=off;
ALTER TABLE workspace_emojis ADD COLUMN width integer, ADD COLUMN height integer, ADD COLUMN image_id text;
-- Canonical read models. Electric 1.x cannot replicate a JOIN VIEW, so their
-- results are cached on the source rows in the same database transaction.
ALTER TABLE messages ADD COLUMN display_data jsonb NOT NULL DEFAULT '{}';
ALTER TABLE notification_events ADD COLUMN display_data jsonb NOT NULL DEFAULT '{}';

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
      'emojis',m.display_data->'emojis') ELSE NULL END,
  'reactions', CASE WHEN n.notification_type='reaction' AND m.deleted_at IS NULL
    THEN COALESCE(m.display_data->'reactions','[]'::jsonb) ELSE '[]'::jsonb END,
  'actor', jsonb_build_object('id',u.id,'user_name',u.user_name,'display_name',u.display_name,
    'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id),
  'channel', to_jsonb(c)
) AS display_data
FROM notification_events n JOIN users u ON u.id=n.actor_id JOIN channels c ON c.id=n.channel_id
LEFT JOIN messages m ON m.id=n.message_id AND m.channel_id=n.channel_id WHERE n.source='channel';

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
  IF TG_OP <> 'DELETE' THEN PERFORM refresh_message_display(NEW.id); END IF;
  -- A reply can move between roots, be deleted, or be restored.
  FOR target IN SELECT DISTINCT parent FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.parent_id END AS parent
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.parent_id END
  ) roots WHERE parent IS NOT NULL ORDER BY parent LOOP
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
  FOR target IN SELECT id FROM notification_events WHERE source='channel' AND message_id=NEW.id ORDER BY id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER message_notifications_display AFTER UPDATE OF content,user_id,display_data,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_notifications_display();

CREATE FUNCTION sync_user_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT id FROM messages WHERE user_id=NEW.id OR id IN (
    SELECT message_id FROM message_reactions WHERE user_id=NEW.id
  ) ORDER BY id LOOP PERFORM refresh_message_display(target); END LOOP;
  FOR target IN SELECT id FROM notification_events WHERE source='channel' AND actor_id=NEW.id ORDER BY id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER users_display AFTER UPDATE OF user_name,display_name,avatar_url,is_bot,app_id ON users FOR EACH ROW EXECUTE FUNCTION sync_user_display();

CREATE FUNCTION sync_emoji_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT id FROM messages WHERE content LIKE '%:%' OR id IN (
    SELECT message_id FROM message_display_reactions WHERE emoji LIKE ':%' OR emoji LIKE 'apub:%'
  ) ORDER BY id LOOP PERFORM refresh_message_display(target); END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER emojis_display AFTER INSERT OR UPDATE OR DELETE ON workspace_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_display();

CREATE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT id FROM notification_events WHERE source='channel' AND channel_id=NEW.id ORDER BY id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
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

CREATE FUNCTION sync_federated_actor_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT DISTINCT post_id FROM apub_reactions WHERE actor_id=NEW.id ORDER BY post_id LOOP
    PERFORM refresh_message_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER federated_actors_display AFTER UPDATE OF user_name,display_name,blocked ON apub_actors FOR EACH ROW EXECUTE FUNCTION sync_federated_actor_display();

-- Backfill while the migration holds the schema lock; old application versions
-- keep writing normally and triggers maintain the projections for them too.
INSERT INTO apub_emoji_images(id,url)
  SELECT DISTINCT emoji->>'image_id',emoji->>'url' FROM apub_reactions
  WHERE jsonb_typeof(emoji)='object' AND emoji->>'image_id' IS NOT NULL AND emoji->>'url' IS NOT NULL
  ON CONFLICT(id) DO NOTHING;
UPDATE messages m SET display_data=v.display_data FROM message_display v WHERE v.id=m.id;
UPDATE notification_events n SET display_data=v.display_data FROM notification_display v WHERE v.id=n.id;
