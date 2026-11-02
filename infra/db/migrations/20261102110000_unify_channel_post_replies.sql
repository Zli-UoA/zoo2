-- Keep channel and Home replies in the same conversation.
CREATE OR REPLACE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE kind text; event_id text := gen_random_uuid()::text;
BEGIN
  IF NEW.channel_id IS NOT NULL THEN
    IF (TG_OP='INSERT' OR NEW.apub AND NEW.deleted_at IS NULL) AND (NEW.visibility='followers' OR NOT EXISTS (
      SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_private AND NOT is_dm AND deleted_at IS NULL
    )) THEN RAISE EXCEPTION 'Home posts require a public channel' USING ERRCODE='23514'; END IF;
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility,parent_id)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility,CASE WHEN EXISTS(SELECT 1 FROM messages parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id) THEN NEW.parent_id END)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility,parent_id=EXCLUDED.parent_id
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility,messages.parent_id)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility,EXCLUDED.parent_id);
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
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET parent_id=COALESCE(NEW.parent_id,parent_id),content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at,parent_id) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC',COALESCE(NEW.parent_id,parent_id));
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND (channel_matches_ltl(c.channel_name,s.ltl_channel_pattern) OR EXISTS(
        SELECT 1 FROM home_posts parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL
      ))
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at,parent_id,visibility,reply_user_id)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC',NEW.parent_id,
        COALESCE((SELECT visibility FROM home_posts WHERE id=NEW.parent_id),'local'),
        (SELECT user_id FROM home_posts WHERE id=NEW.parent_id)) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;

-- Preserve existing channel replies in Home, without changing their audience.
UPDATE home_posts p SET parent_id=m.parent_id
FROM messages m WHERE m.id=p.id AND m.parent_id IS NOT NULL AND p.parent_id IS DISTINCT FROM m.parent_id;

