-- atlas:txmode none

-- An earlier preview already applied the seek migration before parent_id
-- was added to this unmerged PR. Repair that state before building the index.
ALTER TABLE apub_remote_recipients ADD COLUMN IF NOT EXISTS parent_id text;
CREATE OR REPLACE FUNCTION set_apub_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
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
CREATE OR REPLACE FUNCTION sync_apub_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE stripe integer;
BEGIN
  FOR stripe IN SELECT DISTINCT (hashtextextended(parent,0) & 1023)::integer
    FROM (VALUES(OLD.parent_id),(NEW.parent_id)) parents(parent) WHERE parent IS NOT NULL ORDER BY 1
  LOOP
    PERFORM pg_advisory_xact_lock(20880,stripe);
  END LOOP;
  UPDATE apub_remote_recipients r SET parent_id=NEW.parent_id,actor_id=NEW.actor_id,published=NEW.published,post_deleted=NEW.deleted_at IS NOT NULL,
    reply_to_user=EXISTS(SELECT 1 FROM home_posts WHERE id=NEW.parent_id AND user_id=r.user_id)
  WHERE r.post_id=NEW.id AND (r.parent_id,r.actor_id,r.published,r.post_deleted,r.reply_to_user) IS DISTINCT FROM
    (NEW.parent_id,NEW.actor_id,NEW.published,NEW.deleted_at IS NOT NULL,EXISTS(SELECT 1 FROM home_posts WHERE id=NEW.parent_id AND user_id=r.user_id));
  RETURN NULL;
END $$;

-- Repair old preview receipts with the same bounded, ordered-lock backfill.
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
    WHERE r.post_id=b.post_id AND r.user_id=b.user_id AND p.id=r.post_id
      AND (r.parent_id,r.actor_id,r.published,r.post_deleted,r.reply_to_user) IS DISTINCT FROM
      (p.parent_id,p.actor_id,p.published,p.deleted_at IS NOT NULL,EXISTS(SELECT 1 FROM home_posts WHERE id=p.parent_id AND user_id=r.user_id));
    after_post:=batch->-1->>'post_id'; after_user:=batch->-1->>'user_id';
    COMMIT;
  END LOOP;
END $$;
-- Create index "apub_recipient_parent_actor" to table: "apub_remote_recipients"
CREATE INDEX CONCURRENTLY "apub_recipient_parent_actor" ON "apub_remote_recipients" ("user_id", "parent_id", "actor_id") WHERE ((parent_id IS NOT NULL) AND (NOT post_deleted));
-- Create "apub_reply_counts" table
CREATE TABLE "apub_reply_counts" (
  "user_id" text NOT NULL,
  "parent_id" text NOT NULL,
  "actor_id" text NOT NULL,
  "n" bigint NOT NULL DEFAULT 0,
  PRIMARY KEY ("user_id", "parent_id", "actor_id"),
  CONSTRAINT "apub_reply_counts_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);

CREATE FUNCTION maintain_apub_reply_counts() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE old_user text; old_parent text; old_actor text; new_user text; new_parent text; new_actor text; item record;
BEGIN
  IF TG_OP<>'INSERT' AND OLD.parent_id IS NOT NULL AND NOT OLD.post_deleted THEN
    old_user:=OLD.user_id;old_parent:=OLD.parent_id;old_actor:=OLD.actor_id;
  END IF;
  IF TG_OP<>'DELETE' AND NEW.parent_id IS NOT NULL AND NOT NEW.post_deleted THEN
    new_user:=NEW.user_id;new_parent:=NEW.parent_id;new_actor:=NEW.actor_id;
  END IF;
  IF (old_user,old_parent,old_actor) IS NOT DISTINCT FROM (new_user,new_parent,new_actor) THEN RETURN NULL; END IF;
  -- Order actual counter keys, including moves between parents/actors/owners.
  FOR item IN SELECT u,p,a,sum(delta)::bigint AS delta FROM
    (VALUES(old_user,old_parent,old_actor,-1),(new_user,new_parent,new_actor,1)) changes(u,p,a,delta)
    WHERE u IS NOT NULL GROUP BY u,p,a ORDER BY u,p,a
  LOOP
    -- A cascading user deletion removes counters itself; do not recreate them.
    IF EXISTS(SELECT 1 FROM users WHERE id=item.u) THEN
      INSERT INTO apub_reply_counts(user_id,parent_id,actor_id,n) VALUES(item.u,item.p,item.a,item.delta)
      ON CONFLICT(user_id,parent_id,actor_id) DO UPDATE SET n=apub_reply_counts.n+EXCLUDED.n;
    END IF;
  END LOOP;
  RETURN NULL;
END $$;
CREATE TRIGGER apub_reply_counts_change AFTER INSERT OR DELETE OR UPDATE OF user_id,parent_id,actor_id,post_deleted ON apub_remote_recipients FOR EACH ROW EXECUTE FUNCTION maintain_apub_reply_counts();

-- Install the delta trigger first. Seed all pre-existing keys; concurrent new
-- keys already receive their own deltas. Retain zero keys to serialize writers.
INSERT INTO apub_reply_counts(user_id,parent_id,actor_id)
 SELECT DISTINCT user_id,parent_id,actor_id FROM apub_remote_recipients WHERE parent_id IS NOT NULL AND NOT post_deleted
 ORDER BY user_id,parent_id,actor_id
 ON CONFLICT DO NOTHING;
DO $$
DECLARE item record; after_user text; after_parent text; after_actor text; batch jsonb;
BEGIN
 LOOP
  IF after_user IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT user_id,parent_id,actor_id FROM apub_reply_counts ORDER BY user_id,parent_id,actor_id LIMIT 100) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT user_id,parent_id,actor_id FROM apub_reply_counts WHERE (user_id,parent_id,actor_id)>(after_user,after_parent,after_actor) ORDER BY user_id,parent_id,actor_id LIMIT 100) k;
  END IF;
  EXIT WHEN batch IS NULL;
  FOR item IN SELECT * FROM jsonb_to_recordset(batch) AS k(user_id text,parent_id text,actor_id text) ORDER BY user_id,parent_id,actor_id
  LOOP
   -- A delta writer cannot commit its receipt while this counter is locked.
   PERFORM 1 FROM apub_reply_counts WHERE user_id=item.user_id AND parent_id=item.parent_id AND actor_id=item.actor_id FOR UPDATE;
   UPDATE apub_reply_counts c SET n=(SELECT count(*) FROM apub_remote_recipients r WHERE r.user_id=item.user_id AND r.parent_id=item.parent_id AND r.actor_id=item.actor_id AND NOT r.post_deleted)
    WHERE c.user_id=item.user_id AND c.parent_id=item.parent_id AND c.actor_id=item.actor_id;
   after_user:=item.user_id;after_parent:=item.parent_id;after_actor:=item.actor_id;
  END LOOP;
  COMMIT;
 END LOOP;
END $$;
ANALYZE apub_reply_counts;
