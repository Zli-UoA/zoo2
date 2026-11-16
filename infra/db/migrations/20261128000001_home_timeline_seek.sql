-- atlas:txmode none

-- Additive columns and triggers keep old writers compatible before application rollout.
ALTER TABLE "apub_remote_recipients" ADD COLUMN parent_id text, ADD COLUMN "actor_id" text NOT NULL DEFAULT '', ADD COLUMN "published" timestamptz NOT NULL DEFAULT '1970-01-01 00:00:00+00', ADD COLUMN "reply_to_user" boolean NOT NULL DEFAULT false, ADD COLUMN "post_deleted" boolean NOT NULL DEFAULT false;
ALTER TABLE "apub_repost_recipients" ADD COLUMN "actor_id" text NOT NULL DEFAULT '', ADD COLUMN "created_at" timestamptz NOT NULL DEFAULT '1970-01-01 00:00:00+00';

CREATE FUNCTION set_apub_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p apub_remote_posts%ROWTYPE;
BEGIN
  -- SHARE serializes receipt insertion with changes to the post's ordering keys.
  SELECT * INTO p FROM apub_remote_posts WHERE id=NEW.post_id FOR SHARE;
  IF p.parent_id IS NOT NULL THEN
    PERFORM pg_advisory_xact_lock(20880,(hashtextextended(p.parent_id,0) & 1023)::integer);
  END IF;
  NEW.parent_id:=p.parent_id;
  NEW.actor_id:=p.actor_id;
  NEW.published:=p.published;
  NEW.post_deleted:=p.deleted_at IS NOT NULL;
  NEW.reply_to_user:=EXISTS(SELECT 1 FROM home_posts WHERE id=p.parent_id AND user_id=NEW.user_id);
  RETURN NEW;
END $$;
CREATE TRIGGER apub_recipient_order BEFORE INSERT OR UPDATE OF post_id,user_id ON apub_remote_recipients FOR EACH ROW EXECUTE FUNCTION set_apub_recipient_order();
CREATE FUNCTION sync_apub_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.parent_id IS NOT NULL THEN
    PERFORM pg_advisory_xact_lock(20880,(hashtextextended(NEW.parent_id,0) & 1023)::integer);
  END IF;
  UPDATE apub_remote_recipients r SET parent_id=NEW.parent_id,actor_id=NEW.actor_id,published=NEW.published,post_deleted=NEW.deleted_at IS NOT NULL,
    reply_to_user=EXISTS(SELECT 1 FROM home_posts WHERE id=NEW.parent_id AND user_id=r.user_id)
  WHERE r.post_id=NEW.id AND (r.parent_id,r.actor_id,r.published,r.post_deleted,r.reply_to_user) IS DISTINCT FROM
    (NEW.parent_id,NEW.actor_id,NEW.published,NEW.deleted_at IS NOT NULL,EXISTS(SELECT 1 FROM home_posts WHERE id=NEW.parent_id AND user_id=r.user_id));
  RETURN NULL;
END $$;
CREATE TRIGGER apub_post_recipient_order AFTER UPDATE OF actor_id,published,parent_id,deleted_at ON apub_remote_posts FOR EACH ROW EXECUTE FUNCTION sync_apub_recipient_order();
-- Use 1,024 lock stripes: bulk inserts cannot exhaust advisory lock memory.
CREATE FUNCTION sync_home_reply_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE parent text:=COALESCE(NEW.id,OLD.id);
BEGIN
  PERFORM pg_advisory_xact_lock(20880,(hashtextextended(parent,0) & 1023)::integer);
  UPDATE apub_remote_recipients r SET reply_to_user=EXISTS(SELECT 1 FROM home_posts WHERE id=parent AND user_id=r.user_id)
  FROM apub_remote_posts p WHERE p.id=r.post_id AND p.parent_id=parent
    AND r.reply_to_user IS DISTINCT FROM EXISTS(SELECT 1 FROM home_posts WHERE id=parent AND user_id=r.user_id);
  RETURN NULL;
END $$;
CREATE TRIGGER home_reply_recipient_order AFTER INSERT OR DELETE OR UPDATE OF user_id ON home_posts FOR EACH ROW EXECUTE FUNCTION sync_home_reply_recipient_order();
CREATE FUNCTION set_apub_repost_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  SELECT actor_id,created_at INTO NEW.actor_id,NEW.created_at FROM apub_reposts WHERE activity_id=NEW.activity_id FOR SHARE;
  RETURN NEW;
END $$;
CREATE TRIGGER apub_repost_recipient_order BEFORE INSERT OR UPDATE OF activity_id ON apub_repost_recipients FOR EACH ROW EXECUTE FUNCTION set_apub_repost_recipient_order();
CREATE FUNCTION sync_apub_repost_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE apub_repost_recipients SET actor_id=NEW.actor_id,created_at=NEW.created_at
    WHERE activity_id=NEW.activity_id AND (actor_id,created_at) IS DISTINCT FROM (NEW.actor_id,NEW.created_at);
  RETURN NULL;
END $$;
CREATE TRIGGER apub_repost_order AFTER UPDATE OF actor_id,created_at ON apub_reposts FOR EACH ROW EXECUTE FUNCTION sync_apub_repost_recipient_order();


-- Backfill bounded batches while the old application keeps writing. Lock posts,
-- then reply parents, then receipts, matching insert-trigger lock order. Commits
-- bound advisory locks and write blocking to 1,000 receipts at a time.
DO $$
DECLARE batch jsonb; after_post text; after_user text; stripe integer;
BEGIN
  LOOP
    -- Separate the initial page: an OR with a nullable PL/pgSQL parameter
    -- prevents an index range seek once PostgreSQL chooses a generic plan.
    IF after_post IS NULL THEN
      SELECT jsonb_agg(jsonb_build_object('post_id',post_id,'user_id',user_id) ORDER BY post_id,user_id)
        INTO batch FROM (SELECT post_id,user_id FROM apub_remote_recipients ORDER BY post_id,user_id LIMIT 1000) r;
    ELSE
      SELECT jsonb_agg(jsonb_build_object('post_id',post_id,'user_id',user_id) ORDER BY post_id,user_id)
        INTO batch FROM (SELECT post_id,user_id FROM apub_remote_recipients
          WHERE (post_id,user_id)>(after_post,after_user) ORDER BY post_id,user_id LIMIT 1000) r;
    END IF;
    EXIT WHEN batch IS NULL;
    PERFORM p.id FROM apub_remote_posts p WHERE p.id IN (SELECT r.post_id FROM jsonb_to_recordset(batch) AS r(post_id text,user_id text)) ORDER BY p.id FOR SHARE;
    -- Order the actual lock stripe keys, not their unhashed parent IDs.
    FOR stripe IN SELECT DISTINCT (hashtextextended(p.parent_id,0) & 1023)::integer AS lock_key
      FROM apub_remote_posts p WHERE p.parent_id IS NOT NULL AND p.id IN
      (SELECT r.post_id FROM jsonb_to_recordset(batch) AS r(post_id text,user_id text)) ORDER BY lock_key
    LOOP
      PERFORM pg_advisory_xact_lock(20880,stripe);
    END LOOP;
    UPDATE apub_remote_recipients r SET parent_id=p.parent_id,actor_id=p.actor_id,published=p.published,post_deleted=p.deleted_at IS NOT NULL,
      reply_to_user=EXISTS(SELECT 1 FROM home_posts WHERE id=p.parent_id AND user_id=r.user_id)
    FROM apub_remote_posts p,jsonb_to_recordset(batch) AS b(post_id text,user_id text)
    WHERE r.post_id=b.post_id AND r.user_id=b.user_id AND p.id=r.post_id;
    after_post:=batch->-1->>'post_id'; after_user:=batch->-1->>'user_id';
    COMMIT;
  END LOOP;
END $$;
DO $$
DECLARE batch jsonb; after_activity text; after_user text;
BEGIN
  LOOP
    IF after_activity IS NULL THEN
      SELECT jsonb_agg(jsonb_build_object('activity_id',activity_id,'user_id',user_id) ORDER BY activity_id,user_id)
        INTO batch FROM (SELECT activity_id,user_id FROM apub_repost_recipients ORDER BY activity_id,user_id LIMIT 1000) r;
    ELSE
      SELECT jsonb_agg(jsonb_build_object('activity_id',activity_id,'user_id',user_id) ORDER BY activity_id,user_id)
        INTO batch FROM (SELECT activity_id,user_id FROM apub_repost_recipients
          WHERE (activity_id,user_id)>(after_activity,after_user) ORDER BY activity_id,user_id LIMIT 1000) r;
    END IF;
    EXIT WHEN batch IS NULL;
    PERFORM p.activity_id FROM apub_reposts p WHERE p.activity_id IN
      (SELECT r.activity_id FROM jsonb_to_recordset(batch) AS r(activity_id text,user_id text)) ORDER BY p.activity_id FOR SHARE;
    UPDATE apub_repost_recipients r SET actor_id=p.actor_id,created_at=p.created_at
      FROM apub_reposts p,jsonb_to_recordset(batch) AS b(activity_id text,user_id text)
      WHERE r.activity_id=b.activity_id AND r.user_id=b.user_id AND p.activity_id=r.activity_id;
    after_activity:=batch->-1->>'activity_id'; after_user:=batch->-1->>'user_id';
    COMMIT;
  END LOOP;
END $$;


-- Build read indexes without blocking production writes.
CREATE INDEX CONCURRENTLY "apub_recipient_actor_timeline" ON "apub_remote_recipients" ("user_id", "actor_id", "published" DESC, "post_id" DESC) WHERE (NOT post_deleted);
CREATE INDEX CONCURRENTLY "apub_recipient_reply_timeline" ON "apub_remote_recipients" ("user_id", "published" DESC, "post_id" DESC) WHERE (reply_to_user AND (NOT post_deleted));
CREATE INDEX CONCURRENTLY "apub_repost_recipient_timeline" ON "apub_repost_recipients" ("user_id", "actor_id", "created_at" DESC, (('repost:'::text || activity_id)) DESC);
CREATE INDEX CONCURRENTLY "apub_own_repost_timeline" ON "apub_reposts" ("user_id", "created_at" DESC, (('repost:'::text || activity_id)) DESC) WHERE (user_id IS NOT NULL);
CREATE INDEX CONCURRENTLY "apub_public_repost_timeline" ON "apub_reposts" ("created_at" DESC, (('repost:'::text || activity_id)) DESC) WHERE ((user_id IS NOT NULL) AND (visibility = ANY (ARRAY['local'::text, 'public'::text])));
CREATE INDEX CONCURRENTLY "home_live_author_timeline" ON "home_posts" ("user_id", "created_at" DESC, "id" DESC) WHERE (deleted_at IS NULL);
CREATE INDEX CONCURRENTLY "home_live_reply_timeline" ON "home_posts" ("reply_user_id", "created_at" DESC, "id" DESC) WHERE ((deleted_at IS NULL) AND (reply_user_id IS NOT NULL));
CREATE INDEX CONCURRENTLY "notification_activitypub_timeline" ON "notification_events" ("user_id", "created_at" DESC, "id" DESC) WHERE (source = 'activitypub'::text);
CREATE INDEX CONCURRENTLY apub_recipient_reply_actor_timeline ON apub_remote_recipients(user_id,actor_id,published DESC,post_id DESC) WHERE reply_to_user AND NOT post_deleted;

-- Collect statistics for the backfilled ordering and eligibility columns before rollout.
ANALYZE apub_remote_recipients;
ANALYZE apub_repost_recipients;

-- Keep private/unlisted histories out of public and guest ordered scans.
CREATE INDEX CONCURRENTLY home_public_timeline ON home_posts(created_at DESC,id DESC) WHERE deleted_at IS NULL AND visibility IN ('public','local');
CREATE INDEX CONCURRENTLY home_public_author_timeline ON home_posts(user_id,created_at DESC,id DESC) WHERE deleted_at IS NULL AND visibility IN ('public','local');
CREATE INDEX CONCURRENTLY home_public_reply_timeline ON home_posts(reply_user_id,created_at DESC,id DESC) WHERE deleted_at IS NULL AND visibility IN ('public','local') AND reply_user_id IS NOT NULL;
CREATE INDEX CONCURRENTLY home_public_channel_timeline ON home_posts(channel_id,created_at DESC,id DESC) WHERE deleted_at IS NULL AND visibility IN ('public','local') AND channel_id IS NOT NULL;

CREATE INDEX CONCURRENTLY home_public_profile_timeline ON home_posts(user_id,created_at DESC,id DESC) WHERE apub AND visibility='public' AND deleted_at IS NULL;
