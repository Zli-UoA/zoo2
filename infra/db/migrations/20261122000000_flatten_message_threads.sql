-- Keep old writers compatible while ensuring all newly written parent_id values
-- identify the top-level thread root. The original reply target is stored separately.
DO $$ BEGIN LOCK TABLE messages IN ACCESS EXCLUSIVE MODE; END $$;

ALTER TABLE messages ADD COLUMN reply_to_message_id text;

-- Preserve the original target only when a reply targeted another reply. Replies
-- that targeted the root continue to use the ordinary parent_id relationship.
UPDATE messages reply SET reply_to_message_id = parent.id
  FROM messages parent
  WHERE reply.parent_id = parent.id
    AND parent.parent_id IS NOT NULL
    AND reply.reply_to_message_id IS NULL;

-- A flattened thread has one archive and activity state. Preserve any archived
-- descendant as archived and carry the latest activity to the surviving root.
WITH RECURSIVE ancestors(message_id, ancestor_id, ancestor_parent_id, channel_id, visited, depth) AS (
  SELECT id, id, parent_id, channel_id, ARRAY[id], 0 FROM messages
  UNION ALL
  SELECT ancestors.message_id, parent.id, parent.parent_id, ancestors.channel_id,
         ancestors.visited || parent.id, ancestors.depth + 1
    FROM ancestors
    JOIN messages parent ON parent.id = ancestors.ancestor_parent_id
      AND parent.channel_id = ancestors.channel_id
    WHERE parent.id <> ALL(ancestors.visited)
), roots AS (
  SELECT DISTINCT ON (message_id) message_id, ancestor_id AS root_id
    FROM ancestors
    WHERE ancestor_parent_id IS NULL
    ORDER BY message_id, depth DESC, ancestor_id
), resolved AS (
  SELECT messages.id, COALESCE(roots.root_id, messages.id) AS root_id
    FROM messages LEFT JOIN roots ON roots.message_id = messages.id
), thread_states AS (
  SELECT resolved.root_id, max(GREATEST(messages.created_at, messages.thread_activity_at)) AS activity_at,
         max(messages.thread_archive_at) AS archive_at
    FROM messages JOIN resolved ON resolved.id = messages.id
    GROUP BY resolved.root_id
)
UPDATE messages root
  SET thread_activity_at = GREATEST(root.created_at, thread_states.activity_at),
      thread_archive_at = thread_states.archive_at
  FROM thread_states
  WHERE root.id = thread_states.root_id;

-- Resolve each message's highest surviving ancestor. If a malformed cycle has
-- no root, use the message itself as a new root and keep its old target above.
-- Prevent legacy activity logic from treating the normalization update as a
-- newly created reply and reopening archived conversations during migration.
ALTER TABLE messages DISABLE TRIGGER messages_thread_activity_parent_update;
-- Keep the original Home reply chain unchanged while messages are flattened.
ALTER TABLE messages DISABLE TRIGGER channel_home_post_sync;
WITH RECURSIVE ancestors(message_id, ancestor_id, ancestor_parent_id, channel_id, visited, depth) AS (
  SELECT id, id, parent_id, channel_id, ARRAY[id], 0 FROM messages
  UNION ALL
  SELECT ancestors.message_id, parent.id, parent.parent_id, ancestors.channel_id,
         ancestors.visited || parent.id, ancestors.depth + 1
    FROM ancestors
    JOIN messages parent ON parent.id = ancestors.ancestor_parent_id
      AND parent.channel_id = ancestors.channel_id
    WHERE parent.id <> ALL(ancestors.visited)
), roots AS (
  SELECT DISTINCT ON (message_id) message_id, ancestor_id AS root_id
    FROM ancestors
    WHERE ancestor_parent_id IS NULL
    ORDER BY message_id, depth DESC, ancestor_id
), resolved AS (
  SELECT messages.id, COALESCE(roots.root_id, messages.id) AS root_id
    FROM messages LEFT JOIN roots ON roots.message_id = messages.id
)
UPDATE messages target
  SET reply_to_message_id = CASE
        WHEN resolved.root_id = target.id AND target.parent_id IS NOT NULL
          THEN COALESCE(target.reply_to_message_id, target.parent_id)
        ELSE target.reply_to_message_id
      END,
      parent_id = CASE WHEN resolved.root_id = target.id THEN NULL ELSE resolved.root_id END,
      thread_activity_at = CASE WHEN resolved.root_id = target.id THEN target.thread_activity_at ELSE NULL END,
      thread_archive_at = CASE WHEN resolved.root_id = target.id THEN target.thread_archive_at ELSE NULL END
  FROM resolved
  WHERE target.id = resolved.id
    AND (target.parent_id IS DISTINCT FROM
      CASE WHEN resolved.root_id = target.id THEN NULL ELSE resolved.root_id END
      OR resolved.root_id <> target.id
        AND (target.thread_activity_at IS NOT NULL OR target.thread_archive_at IS NOT NULL));
ALTER TABLE messages ENABLE TRIGGER messages_thread_activity_parent_update;
ALTER TABLE messages ENABLE TRIGGER channel_home_post_sync;

CREATE FUNCTION normalize_message_thread_parent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE requested_parent text; requested_parent_parent text; root_id text;
BEGIN
  IF NEW.parent_id IS NULL THEN RETURN NEW; END IF;
  requested_parent := NEW.parent_id;
  IF NEW.id = requested_parent THEN
    RAISE EXCEPTION 'a message cannot reply to itself' USING ERRCODE = '23514';
  END IF;
  SELECT parent_id INTO requested_parent_parent FROM messages
    WHERE id = requested_parent AND channel_id = NEW.channel_id;
  IF NOT FOUND THEN
    IF EXISTS (SELECT 1 FROM messages WHERE id = requested_parent) THEN
      RAISE EXCEPTION 'reply target belongs to another channel' USING ERRCODE = '23514';
    END IF;
    -- 存在しない ID は既存の外部キー制約に拒否させる。
    RETURN NEW;
  END IF;
  IF requested_parent_parent IS NOT NULL AND NEW.reply_to_message_id IS NULL THEN
    NEW.reply_to_message_id := requested_parent;
  END IF;
  WITH RECURSIVE ancestors(id, parent_id, visited) AS (
    SELECT id, parent_id, ARRAY[id] FROM messages
      WHERE id = requested_parent AND channel_id = NEW.channel_id
    UNION ALL
    SELECT parent.id, parent.parent_id, ancestors.visited || parent.id
      FROM ancestors JOIN messages parent
        ON parent.id = ancestors.parent_id AND parent.channel_id = NEW.channel_id
      WHERE parent.id <> ALL(ancestors.visited)
  )
  SELECT id INTO root_id FROM ancestors WHERE parent_id IS NULL LIMIT 1;
  IF root_id IS NULL THEN
    -- 祖先を確定できない場合は新規投稿を親として保存し、辿ろうとした ID を残す。
    NEW.reply_to_message_id := COALESCE(NEW.reply_to_message_id, requested_parent);
    NEW.parent_id := NULL;
  ELSE
    NEW.parent_id := root_id;
  END IF;
  RETURN NEW;
END $$;

CREATE TRIGGER messages_normalize_thread_parent BEFORE INSERT OR UPDATE OF parent_id ON messages
  FOR EACH ROW EXECUTE FUNCTION normalize_message_thread_parent();

-- Home keeps the actual reply chain for ActivityPub while messages.parent_id
-- identifies the flattened Zoo thread. Keep those relationships independent.
CREATE OR REPLACE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE kind text; event_id text := gen_random_uuid()::text;
BEGIN
  IF NEW.channel_id IS NOT NULL THEN
    IF (TG_OP='INSERT' OR NEW.apub AND NEW.deleted_at IS NULL) AND (NEW.visibility='followers' OR NOT EXISTS (
      SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_private AND NOT is_dm AND deleted_at IS NULL
    )) THEN RAISE EXCEPTION 'Home posts require a public channel' USING ERRCODE='23514'; END IF;
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility,parent_id,reply_to_message_id)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility,CASE WHEN EXISTS(SELECT 1 FROM messages parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id) THEN NEW.parent_id END,CASE WHEN EXISTS(SELECT 1 FROM messages parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id) THEN NEW.parent_id END)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility,parent_id=EXCLUDED.parent_id,reply_to_message_id=EXCLUDED.reply_to_message_id
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility,messages.parent_id,messages.reply_to_message_id)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility,EXCLUDED.parent_id,EXCLUDED.reply_to_message_id);
  END IF;
  IF TG_OP='INSERT' THEN
    IF NEW.apub AND NEW.deleted_at IS NULL THEN kind:='Create'; END IF;
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND NOT OLD.apub THEN kind:='Create';
  ELSIF OLD.apub AND OLD.deleted_at IS NULL AND (NOT NEW.apub OR NEW.deleted_at IS NOT NULL) THEN kind:='Delete';
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND (NEW.content,NEW.visibility) IS DISTINCT FROM (OLD.content,OLD.visibility) THEN kind:='Update';
  END IF;
  IF kind IS NOT NULL THEN
    INSERT INTO apub_deliveries(id,user_id,actor_id,payload)
      SELECT gen_random_uuid()::text,NEW.user_id,a.id,
        json_build_object('zoo_post',CASE WHEN kind='Delete' THEN row_to_json(OLD) ELSE row_to_json(NEW) END,'type',kind,'activity_id',event_id)::text
      FROM apub_actors a
      WHERE NOT a.blocked AND (a.id=NEW.reply_actor OR EXISTS(SELECT 1 FROM apub_follows f WHERE f.actor_id=a.id AND f.user_id=NEW.user_id AND f.direction='incoming' AND f.accepted));
  END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION sync_channel_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE reply_target text := COALESCE(NEW.reply_to_message_id, NEW.parent_id);
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET parent_id=reply_target,content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at,parent_id) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC',reply_target);
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND (channel_matches_ltl(c.channel_name,s.ltl_channel_pattern) OR EXISTS(
        SELECT 1 FROM home_posts parent WHERE parent.id=reply_target AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL
      ))
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at,parent_id,visibility,reply_user_id)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC',reply_target,
        COALESCE((SELECT visibility FROM home_posts WHERE id=reply_target),'local'),
        (SELECT user_id FROM home_posts WHERE id=reply_target)) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;
