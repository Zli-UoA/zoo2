ALTER TABLE home_posts DROP CONSTRAINT home_posts_content_check;
ALTER TABLE home_posts ADD CONSTRAINT home_posts_content_check CHECK(char_length(content) <= 40000);
ALTER TABLE home_posts ADD COLUMN apub boolean NOT NULL DEFAULT false,
  ADD COLUMN visibility text NOT NULL DEFAULT 'local' CHECK (visibility IN ('local','public','unlisted','followers')),
  ADD COLUMN channel_id text REFERENCES channels(id) ON DELETE SET NULL,
  ADD COLUMN updated_at timestamptz,
  ADD COLUMN deleted_at timestamptz,
  ADD CONSTRAINT home_posts_audience CHECK (apub = (visibility <> 'local')),
  ADD CONSTRAINT home_posts_channel_audience CHECK (channel_id IS NULL OR visibility <> 'followers');
ALTER TABLE messages ADD COLUMN home_post_id text UNIQUE,
  ADD COLUMN apub boolean NOT NULL DEFAULT false,
  ADD COLUMN apub_visibility text;
CREATE TABLE activitypub_settings (
  id boolean PRIMARY KEY DEFAULT true CHECK(id),
  ltl_channel_pattern text NOT NULL DEFAULT '' CHECK(length(ltl_channel_pattern)<=200)
);
INSERT INTO activitypub_settings DEFAULT VALUES;
CREATE TABLE apub_keys (
  user_id text PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  private_key text NOT NULL,
  public_key text NOT NULL
);
CREATE TABLE apub_actors (
  id text PRIMARY KEY,
  inbox text NOT NULL,
  followers_url text NOT NULL,
  user_name text NOT NULL,
  display_name text NOT NULL,
  key_id text NOT NULL,
  public_key text NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now(),
  blocked boolean NOT NULL DEFAULT false
);
CREATE TABLE apub_follows (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  direction text NOT NULL CHECK(direction IN ('incoming','outgoing')),
  activity_id text NOT NULL UNIQUE,
  accepted boolean NOT NULL DEFAULT false,
  PRIMARY KEY(user_id,actor_id,direction)
);
CREATE TABLE apub_deliveries (
  id text PRIMARY KEY,
  sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  payload text NOT NULL,
  attempts integer NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  lease_until timestamptz,
  delivered_at timestamptz,
  last_status integer,
  failed boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_apub_delivery_pending ON apub_deliveries(next_attempt_at) WHERE delivered_at IS NULL AND NOT failed;
CREATE TABLE apub_inbox_receipts (
  id text NOT NULL,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL,
  PRIMARY KEY(id,user_id),
  received_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE apub_remote_posts (
  id text PRIMARY KEY,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  content text NOT NULL,
  visibility text NOT NULL CHECK(visibility IN ('public','unlisted','followers')),
  published timestamptz NOT NULL,
  updated_at timestamptz NOT NULL,
  deleted_at timestamptz
);
CREATE TABLE apub_remote_recipients (
  post_id text NOT NULL REFERENCES apub_remote_posts(id) ON DELETE CASCADE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(post_id,user_id)
);
CREATE INDEX idx_apub_remote_published ON apub_remote_posts(published DESC,id DESC);

-- Synchronize both views inside the originating transaction. Comparisons prevent recursion.
CREATE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
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
      SELECT gen_random_uuid()::text,NEW.user_id,f.actor_id,
        json_build_object('zoo_post',CASE WHEN kind='Delete' THEN row_to_json(OLD) ELSE row_to_json(NEW) END,'type',kind,'activity_id',event_id)::text
      FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id
      WHERE f.user_id=NEW.user_id AND f.direction='incoming' AND f.accepted AND NOT a.blocked;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER home_post_sync AFTER INSERT OR UPDATE ON home_posts FOR EACH ROW EXECUTE FUNCTION sync_home_post();
CREATE FUNCTION sync_channel_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC');
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND s.ltl_channel_pattern<>'' AND c.channel_name LIKE s.ltl_channel_pattern ESCAPE ''
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC') ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channel_home_post_sync AFTER INSERT OR UPDATE ON messages FOR EACH ROW EXECUTE FUNCTION sync_channel_home_post();
-- Already federated content cannot silently become a private channel conversation.
CREATE FUNCTION protect_federated_channel() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.is_private AND NOT OLD.is_private AND EXISTS(SELECT 1 FROM home_posts WHERE channel_id=NEW.id AND apub AND deleted_at IS NULL) THEN
    RAISE EXCEPTION 'Delete federated posts before making this channel private' USING ERRCODE='23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER protect_federated_channel_privacy BEFORE UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION protect_federated_channel();

CREATE FUNCTION delete_channel_home_posts() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL THEN
    UPDATE home_posts SET deleted_at=now(),updated_at=now() WHERE channel_id=NEW.id AND deleted_at IS NULL;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER delete_channel_home_posts AFTER UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION delete_channel_home_posts();
