ALTER TABLE channels ADD COLUMN parent_channel_id text;
ALTER TABLE channels ADD COLUMN thread_title text;
ALTER TABLE channels ADD COLUMN thread_archived_at timestamp(3);
ALTER TABLE channels ADD COLUMN thread_activity_at timestamp(3);
ALTER TABLE channels ADD CONSTRAINT channels_parent_channel_id_fkey
  FOREIGN KEY (parent_channel_id) REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE;
CREATE INDEX idx_channels_parent_thread ON channels (parent_channel_id) WHERE parent_channel_id IS NOT NULL;

ALTER TABLE messages ADD COLUMN reply_to_message_id text;
ALTER TABLE messages ADD CONSTRAINT messages_reply_to_message_id_fkey
  FOREIGN KEY (reply_to_message_id) REFERENCES messages (id) ON UPDATE CASCADE ON DELETE RESTRICT;
CREATE INDEX idx_messages_reply_to_message_id
  ON messages (reply_to_message_id) WHERE reply_to_message_id IS NOT NULL;

CREATE TABLE threads (
  channel_id text NOT NULL,
  parent_channel_id text NOT NULL,
  title text NOT NULL,
  source_message_id text,
  archived_at timestamp(3),
  activity_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT threads_pkey PRIMARY KEY (channel_id),
  CONSTRAINT threads_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT threads_parent_channel_id_fkey FOREIGN KEY (parent_channel_id) REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT threads_source_message_id_fkey FOREIGN KEY (source_message_id) REFERENCES messages (id) ON UPDATE CASCADE ON DELETE SET NULL
);
CREATE INDEX idx_threads_parent_activity ON threads (parent_channel_id, archived_at, activity_at DESC);
CREATE INDEX idx_threads_source_message ON threads (source_message_id) WHERE source_message_id IS NOT NULL;

CREATE FUNCTION update_channel_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP = 'UPDATE' AND NEW.channel_id IS NOT DISTINCT FROM OLD.channel_id
     AND NEW.deleted_at IS NOT DISTINCT FROM OLD.deleted_at THEN RETURN NULL; END IF;
  UPDATE threads SET
    activity_at = GREATEST(activity_at, NEW.created_at),
    archived_at = CASE WHEN NEW.created_at >= archived_at
      OR NEW.created_at >= date_trunc('milliseconds', transaction_timestamp() AT TIME ZONE 'UTC')
      THEN NULL ELSE archived_at END
  WHERE channel_id = NEW.channel_id;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_thread_channel_activity
  AFTER INSERT OR UPDATE OF channel_id, deleted_at ON messages
  FOR EACH ROW EXECUTE FUNCTION update_channel_thread_activity();

CREATE FUNCTION sync_thread_channel_projection() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE channels SET thread_archived_at=NEW.archived_at, thread_activity_at=NEW.activity_at
  WHERE id=NEW.channel_id;
  RETURN NULL;
END $$;
CREATE TRIGGER threads_channel_projection
  AFTER INSERT OR UPDATE OF archived_at, activity_at ON threads
  FOR EACH ROW EXECUTE FUNCTION sync_thread_channel_projection();

-- Stage one is additive: keep existing reply rows in their current channel so old
-- backends can continue serving legacy thread panes while this schema is deployed.
-- Record legacy roots and allow old backend writes during the rolling upgrade.
CREATE TABLE legacy_thread_roots (
  root_message_id text PRIMARY KEY REFERENCES messages(id) ON UPDATE CASCADE ON DELETE CASCADE
);
WITH RECURSIVE roots AS (
  SELECT root.id FROM messages root
  WHERE root.parent_id IS NULL
    AND EXISTS (SELECT 1 FROM messages child WHERE child.parent_id=root.id)
)
INSERT INTO legacy_thread_roots(root_message_id)
SELECT id FROM roots ON CONFLICT DO NOTHING;

CREATE FUNCTION register_legacy_thread_root() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE root_id text;
BEGIN
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  WITH RECURSIVE ancestors(id,parent_id,channel_id) AS (
    SELECT parent.id,parent.parent_id,parent.channel_id FROM messages parent WHERE parent.id=NEW.parent_id
    UNION ALL
    SELECT parent.id,parent.parent_id,parent.channel_id
    FROM ancestors child JOIN messages parent ON parent.id=child.parent_id
    WHERE child.parent_id IS NOT NULL AND child.channel_id=NEW.channel_id
  )
  SELECT id INTO root_id FROM ancestors
  WHERE parent_id IS NULL AND channel_id=NEW.channel_id LIMIT 1;
  IF root_id IS NOT NULL THEN
    INSERT INTO legacy_thread_roots(root_message_id) VALUES(root_id) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_register_legacy_thread_root_insert
  AFTER INSERT ON messages
  FOR EACH ROW EXECUTE FUNCTION register_legacy_thread_root();
CREATE TRIGGER messages_register_legacy_thread_root_update
  AFTER UPDATE OF parent_id ON messages
  FOR EACH ROW EXECUTE FUNCTION register_legacy_thread_root();

CREATE OR REPLACE VIEW message_display AS
SELECT m.id, jsonb_build_object(
  'author', (SELECT jsonb_build_object('id',u.id,'user_name',u.user_name,
    'display_name',u.display_name,'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id)
    FROM users u WHERE u.id=m.user_id),
  'emojis', COALESCE((SELECT jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id))
    FROM display_workspace_emojis e CROSS JOIN LATERAL (
      SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
    ) names WHERE strpos(m.content,':'||names.name||':')>0 OR EXISTS (
      SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND r.emoji=':'||names.name||':'
    )), '{}'::jsonb) || COALESCE((SELECT jsonb_object_agg(r.emoji,r.custom_emoji) FROM message_display_reactions r WHERE r.message_id=m.id AND r.custom_emoji IS NOT NULL AND r.emoji LIKE 'apub:%'),'{}'::jsonb),
  'attachments', COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.created_at,a.id)
    FROM attachments a WHERE a.message_id=m.id),'[]'::jsonb),
  'reactions', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.created_at,r.id)
    FROM message_display_reactions r WHERE r.message_id=m.id),'[]'::jsonb),
  'reply_target', (SELECT jsonb_build_object('id',target.id,'user_id',target.user_id,'channel_id',target.channel_id,
    'content',CASE WHEN target.deleted_at IS NULL THEN target.content ELSE NULL END,
    'deleted_at',target.deleted_at,'author',(SELECT jsonb_build_object('user_name',u.user_name,
      'display_name',u.display_name,'avatar_url',u.avatar_url) FROM users u WHERE u.id=target.user_id))
    FROM messages target WHERE target.id=m.reply_to_message_id),
  'reply_count', (SELECT count(*) FROM messages reply WHERE (reply.reply_to_message_id=m.id OR (reply.reply_to_message_id IS NULL AND reply.parent_id=m.id)) AND reply.deleted_at IS NULL)
) AS display_data FROM messages m;

DROP TRIGGER messages_display_update ON messages;
CREATE TRIGGER messages_display_update AFTER UPDATE OF content,user_id,parent_id,reply_to_message_id,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();

CREATE FUNCTION refresh_reply_target_previews() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE messages reply SET display_data=source.display_data
  FROM message_display source
  WHERE reply.reply_to_message_id=NEW.id AND source.id=reply.id;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_reply_target_preview_update
  AFTER UPDATE OF content,user_id,deleted_at ON messages
  FOR EACH ROW EXECUTE FUNCTION refresh_reply_target_previews();

UPDATE messages m SET display_data=source.display_data
FROM message_display source
WHERE m.id=source.id AND m.reply_to_message_id IS NOT NULL
  AND m.display_data IS DISTINCT FROM source.display_data;
