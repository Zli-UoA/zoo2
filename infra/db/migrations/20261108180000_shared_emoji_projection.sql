-- Additive read projection. Atlas diff requires authentication in this environment.

-- Shared service read projection; management and ownership remain in the emoji service.
CREATE TABLE shared_emoji_source(id text PRIMARY KEY CHECK (id='workspace'),url text);
CREATE TABLE shared_emojis(
  name text PRIMARY KEY,url text NOT NULL,category text NOT NULL DEFAULT '',
  aliases jsonb NOT NULL DEFAULT '[]'::jsonb,width integer,height integer,image_id text,
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE VIEW display_workspace_emojis AS
SELECT width,height,image_id,name,url,''::text AS created_by,updated_at,category,aliases FROM shared_emojis
WHERE EXISTS(SELECT 1 FROM shared_emoji_source WHERE id='workspace' AND url IS NOT NULL)
UNION ALL
SELECT width,height,image_id,name,url,created_by,updated_at,category,aliases FROM workspace_emojis
WHERE NOT EXISTS(SELECT 1 FROM shared_emoji_source WHERE id='workspace' AND url IS NOT NULL);


CREATE OR REPLACE VIEW message_display_reactions AS
SELECT r.id,r.message_id,r.user_id,r.emoji,r.created_at,u.user_name,u.display_name,NULL::jsonb AS custom_emoji
FROM message_reactions r JOIN users u ON u.id=r.user_id
UNION ALL
SELECT r.activity_id,p.id,r.actor_id,
  CASE WHEN jsonb_typeof(r.emoji)='object' THEN COALESCE(
    (SELECT ':'||e.name||':' FROM display_workspace_emojis e WHERE e.image_id=r.emoji->>'image_id' ORDER BY e.name LIMIT 1),
    'apub:'||(r.emoji->>'image_id')||':'||(r.emoji->>'name')) ELSE r.reaction END,
  r.created_at AT TIME ZONE 'UTC',a.user_name||'@'||split_part(a.id,'/',3),a.display_name,r.emoji
FROM apub_reactions r JOIN home_posts p ON p.id=r.post_id JOIN messages m ON m.id=p.id
JOIN apub_actors a ON a.id=r.actor_id
WHERE p.channel_id IS NOT NULL AND p.deleted_at IS NULL AND m.deleted_at IS NULL
  AND r.user_id IS NULL AND NOT a.blocked;


CREATE OR REPLACE FUNCTION message_content_emojis(content text) RETURNS jsonb LANGUAGE sql STABLE AS $$
  SELECT COALESCE(jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id)), '{}'::jsonb)
  FROM display_workspace_emojis e CROSS JOIN LATERAL (
    SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
  ) names WHERE strpos(content,':'||names.name||':')>0;
$$;

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
  'reply_count', (SELECT count(*) FROM messages reply WHERE reply.parent_id=m.id AND reply.deleted_at IS NULL)
) AS display_data FROM messages m;


CREATE TRIGGER shared_emojis_display_update AFTER UPDATE ON shared_emojis REFERENCING OLD TABLE AS old_emojis NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_display();
CREATE TRIGGER shared_emojis_display_insert AFTER INSERT ON shared_emojis REFERENCING NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_insert_display();
CREATE TRIGGER shared_emojis_display_delete AFTER DELETE ON shared_emojis REFERENCING OLD TABLE AS old_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_delete_display();
CREATE FUNCTION sync_shared_emoji_source_display() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP='UPDATE' AND OLD.url IS NOT DISTINCT FROM NEW.url THEN RETURN NULL; END IF;
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM (
    SELECT name,aliases,image_id FROM workspace_emojis UNION ALL SELECT name,aliases,image_id FROM shared_emojis
  ) e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE TRIGGER shared_emoji_source_display AFTER INSERT OR UPDATE ON shared_emoji_source FOR EACH ROW EXECUTE FUNCTION sync_shared_emoji_source_display();
