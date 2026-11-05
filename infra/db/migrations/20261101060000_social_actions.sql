ALTER TABLE home_posts ADD COLUMN parent_id text;
ALTER TABLE home_posts ADD COLUMN parent_uri text;
ALTER TABLE home_posts ADD COLUMN reply_actor text;
ALTER TABLE home_posts ADD COLUMN reply_user_id text REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE apub_remote_posts ADD COLUMN parent_id text;
ALTER TABLE apub_remote_posts ADD COLUMN emojis jsonb NOT NULL DEFAULT '[]';
ALTER TABLE apub_actors ADD COLUMN emojis jsonb NOT NULL DEFAULT '[]';
ALTER TABLE apub_reactions ADD COLUMN reaction text NOT NULL DEFAULT '❤️';
ALTER TABLE apub_reactions ADD COLUMN emoji jsonb;
CREATE TABLE apub_emoji_images (
  id text PRIMARY KEY,
  url text NOT NULL UNIQUE CHECK(length(url)<=2048)
);
CREATE TABLE apub_emoji_catalog (
  origin text PRIMARY KEY,
  software text NOT NULL,
  emojis jsonb NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE apub_reposts (
  post_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text REFERENCES users(id) ON DELETE CASCADE,
  activity_id text NOT NULL UNIQUE,
  activity_json text NOT NULL,
  visibility text NOT NULL CHECK(visibility IN ('local','public','unlisted','followers')),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(post_id,actor_id)
);
CREATE TABLE apub_repost_recipients (
  activity_id text NOT NULL REFERENCES apub_reposts(activity_id) ON DELETE CASCADE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(activity_id,user_id)
);
ALTER TABLE apub_notifications DROP CONSTRAINT apub_notifications_kind_check;
ALTER TABLE apub_notifications ADD CONSTRAINT apub_notifications_kind_check CHECK(kind IN ('follow_request','follow_accepted','like','reply','repost'));
CREATE INDEX home_reply_parent ON home_posts(parent_id);
CREATE INDEX remote_reply_parent ON apub_remote_posts(parent_id);
CREATE TRIGGER apub_reposts_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_reposts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_repost_recipients_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_repost_recipients FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
ALTER TABLE apub_notifications ADD COLUMN reaction text;
ALTER TABLE apub_notifications ADD COLUMN emoji jsonb;

CREATE OR REPLACE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE kind text; event_id text := gen_random_uuid()::text;
BEGIN
  IF NEW.channel_id IS NOT NULL THEN
    IF (TG_OP='INSERT' OR NEW.apub AND NEW.deleted_at IS NULL) AND (NEW.visibility='followers' OR NOT EXISTS (
      SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_private AND NOT is_dm AND deleted_at IS NULL
    )) THEN RAISE EXCEPTION 'Home posts require a public channel' USING ERRCODE='23514'; END IF;
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility);
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
