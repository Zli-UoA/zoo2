-- atlas:txmode none

-- Create index "apub_blocked_actors" to table: "apub_actors"
CREATE INDEX CONCURRENTLY "apub_blocked_actors" ON "apub_actors" ("id") WHERE blocked;
-- Create index "apub_reaction_owner_post" to table: "apub_reactions"
CREATE INDEX CONCURRENTLY "apub_reaction_owner_post" ON "apub_reactions" ("post_id", "user_id") WHERE (user_id IS NOT NULL);
-- Modify "apub_repost_recipients" table
ALTER TABLE "apub_repost_recipients" ADD COLUMN "post_id" text NOT NULL DEFAULT '', ADD COLUMN "private_counted" boolean NOT NULL DEFAULT false;
-- Create index "apub_repost_owner_post" to table: "apub_reposts"
CREATE INDEX CONCURRENTLY "apub_repost_owner_post" ON "apub_reposts" ("user_id", "post_id") WHERE (user_id IS NOT NULL);
-- Create "home_reaction_totals" table
CREATE TABLE "home_reaction_totals" (
  "post_id" text NOT NULL,
  "source_kind" smallint NOT NULL,
  "group_key" text NOT NULL,
  "reaction" text NOT NULL,
  "emoji" jsonb NULL,
  "n" bigint NOT NULL DEFAULT 0,
  PRIMARY KEY ("post_id", "source_kind", "group_key")
);
-- Create index "home_reaction_totals_live" to table: "home_reaction_totals"
CREATE INDEX CONCURRENTLY "home_reaction_totals_live" ON "home_reaction_totals" ("post_id", "source_kind", "group_key") WHERE (n > 0);
-- Create "home_repost_totals" table
CREATE TABLE "home_repost_totals" (
  "post_id" text NOT NULL,
  "n" bigint NOT NULL DEFAULT 0,
  PRIMARY KEY ("post_id")
);
-- Create "home_private_repost_totals" table
CREATE TABLE "home_private_repost_totals" (
  "user_id" text NOT NULL,
  "post_id" text NOT NULL,
  "n" bigint NOT NULL DEFAULT 0,
  PRIMARY KEY ("user_id", "post_id"),
  CONSTRAINT "home_private_repost_totals_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);

-- Explicit UTF-8 keeps keys independent of client_encoding; JSONB canonically
-- encodes both fixed argument types. Hashing keeps long custom URIs out of keys.
CREATE FUNCTION home_reaction_key(reaction text, emoji jsonb) RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
 SELECT encode(sha256(convert_to(jsonb_build_array(reaction,emoji)::text,'UTF8')),'hex');
$$;
CREATE INDEX CONCURRENTLY apub_reaction_group_order ON apub_reactions(post_id,(user_id IS NULL),home_reaction_key(reaction,emoji),created_at);
CREATE INDEX CONCURRENTLY message_reaction_group_order ON message_reactions(message_id,home_reaction_key(emoji,NULL::jsonb),created_at);

CREATE FUNCTION maintain_home_reaction_totals() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE before_row jsonb; after_row jsonb; item record;
BEGIN
 IF TG_OP<>'INSERT' THEN before_row:=to_jsonb(OLD); END IF;
 IF TG_OP<>'DELETE' THEN after_row:=to_jsonb(NEW); END IF;
 -- The existing display triggers lock messages too. Take that lock before
 -- the shared counter, including linked-post reactions from either API.
 PERFORM id FROM messages WHERE id IN (
  SELECT COALESCE(r->>'post_id',r->>'message_id') FROM (VALUES(before_row),(after_row)) v(r)
 ) ORDER BY id FOR NO KEY UPDATE;
 FOR item IN
  WITH changes AS (SELECT r,d FROM (VALUES(before_row,-1),(after_row,1)) v(r,d) WHERE r IS NOT NULL),
  entries AS (SELECT CASE WHEN TG_TABLE_NAME='message_reactions' THEN r->>'message_id' ELSE r->>'post_id' END AS post_id,
   CASE WHEN TG_TABLE_NAME='message_reactions' THEN 2 WHEN r->>'user_id' IS NULL THEN 0 ELSE 1 END::smallint AS kind,
   CASE WHEN TG_TABLE_NAME='message_reactions' THEN r->>'emoji' ELSE r->>'reaction' END AS reaction,
   CASE WHEN TG_TABLE_NAME='message_reactions' THEN NULL::jsonb ELSE NULLIF(r->'emoji','null'::jsonb) END AS emoji,d FROM changes)
  SELECT post_id,kind,home_reaction_key(reaction,emoji) AS key,reaction,emoji,sum(d)::bigint AS delta
   FROM entries GROUP BY post_id,kind,key,reaction,emoji HAVING sum(d)<>0 ORDER BY post_id,kind,key
 LOOP
  INSERT INTO home_reaction_totals(post_id,source_kind,group_key,reaction,emoji,n)
   VALUES(item.post_id,item.kind,item.key,item.reaction,item.emoji,item.delta)
   ON CONFLICT(post_id,source_kind,group_key) DO UPDATE SET n=home_reaction_totals.n+EXCLUDED.n;
 END LOOP;
 RETURN NULL;
END $$;
CREATE TRIGGER home_reaction_totals_change AFTER INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH ROW EXECUTE FUNCTION maintain_home_reaction_totals();
CREATE TRIGGER home_message_reaction_totals_change AFTER INSERT OR UPDATE OR DELETE ON message_reactions FOR EACH ROW EXECUTE FUNCTION maintain_home_reaction_totals();

CREATE FUNCTION maintain_home_repost_totals() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE before_row jsonb; after_row jsonb; item record; stripe integer;
BEGIN
 IF TG_OP<>'INSERT' THEN before_row:=to_jsonb(OLD); END IF;
 IF TG_OP<>'DELETE' THEN after_row:=to_jsonb(NEW); END IF;
 -- Public and per-recipient deltas for a post share a transaction lock.
 -- No request can hold private counter keys in opposite recipient orders.
 FOR stripe IN SELECT DISTINCT (hashtextextended(r->>'post_id',0) & 1023)::integer
  FROM (VALUES(before_row),(after_row)) v(r) WHERE r IS NOT NULL ORDER BY 1
 LOOP
  PERFORM pg_advisory_xact_lock(20881,stripe);
 END LOOP;
 FOR item IN
  WITH changes AS (SELECT r,d FROM (VALUES(before_row,-1),(after_row,1)) v(r,d) WHERE r IS NOT NULL)
  SELECT r->>'post_id' AS post_id,CASE WHEN TG_TABLE_NAME='apub_reposts' THEN NULL::text ELSE r->>'user_id' END AS user_id,sum(d)::bigint AS delta
   FROM changes WHERE CASE WHEN TG_TABLE_NAME='apub_reposts' THEN r->>'visibility' IN ('public','local') ELSE (r->>'private_counted')::boolean END
   GROUP BY post_id,user_id HAVING sum(d)<>0 ORDER BY user_id,post_id
 LOOP
  IF item.user_id IS NULL THEN
   INSERT INTO home_repost_totals(post_id,n) VALUES(item.post_id,item.delta)
    ON CONFLICT(post_id) DO UPDATE SET n=home_repost_totals.n+EXCLUDED.n;
  ELSIF EXISTS(SELECT 1 FROM users WHERE id=item.user_id) THEN
   INSERT INTO home_private_repost_totals(user_id,post_id,n) VALUES(item.user_id,item.post_id,item.delta)
    ON CONFLICT(user_id,post_id) DO UPDATE SET n=home_private_repost_totals.n+EXCLUDED.n;
  END IF;
 END LOOP;
 RETURN NULL;
END $$;
CREATE TRIGGER home_repost_totals_change AFTER INSERT OR UPDATE OR DELETE ON apub_reposts FOR EACH ROW EXECUTE FUNCTION maintain_home_repost_totals();

CREATE OR REPLACE FUNCTION set_apub_repost_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p apub_reposts%ROWTYPE;
BEGIN
 SELECT * INTO p FROM apub_reposts WHERE activity_id=NEW.activity_id FOR SHARE;
 NEW.actor_id:=p.actor_id; NEW.created_at:=p.created_at; NEW.post_id:=p.post_id;
 NEW.private_counted:=p.visibility NOT IN ('public','local') AND p.user_id IS DISTINCT FROM NEW.user_id;
 RETURN NEW;
END $$;
DROP TRIGGER apub_repost_recipient_order ON apub_repost_recipients;
CREATE TRIGGER apub_repost_recipient_order BEFORE INSERT OR UPDATE OF activity_id,user_id ON apub_repost_recipients FOR EACH ROW EXECUTE FUNCTION set_apub_repost_recipient_order();
CREATE OR REPLACE FUNCTION sync_apub_repost_recipient_order() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE apub_repost_recipients r SET actor_id=NEW.actor_id,created_at=NEW.created_at,post_id=NEW.post_id,
  private_counted=NEW.visibility NOT IN ('public','local') AND NEW.user_id IS DISTINCT FROM r.user_id
  WHERE activity_id=NEW.activity_id AND (r.actor_id,r.created_at,r.post_id,r.private_counted) IS DISTINCT FROM
   (NEW.actor_id,NEW.created_at,NEW.post_id,NEW.visibility NOT IN ('public','local') AND NEW.user_id IS DISTINCT FROM r.user_id);
 RETURN NULL;
END $$;
DROP TRIGGER apub_repost_order ON apub_reposts;
CREATE TRIGGER apub_repost_order AFTER UPDATE OF actor_id,created_at,post_id,user_id,visibility ON apub_reposts FOR EACH ROW EXECUTE FUNCTION sync_apub_repost_recipient_order();

CREATE INDEX CONCURRENTLY apub_private_repost_recipient_post ON apub_repost_recipients(user_id,post_id) WHERE private_counted;

CREATE OR REPLACE FUNCTION sync_federated_reaction_display() RETURNS trigger LANGUAGE plpgsql AS $$
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
    IF EXISTS(SELECT 1 FROM messages WHERE id=target) THEN
      PERFORM refresh_message_display(target);
      INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
    END IF;
  END LOOP;
  RETURN NULL;
END;
$$;

-- Fill the new private-audience metadata with ordered source locks, before
-- enabling its delta trigger and seeding/recounting the affected keys.
DO $$
DECLARE batch jsonb; after_activity text; after_user text;
BEGIN
 LOOP
  IF after_activity IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT activity_id,user_id FROM apub_repost_recipients ORDER BY activity_id,user_id LIMIT 1000) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT activity_id,user_id FROM apub_repost_recipients WHERE (activity_id,user_id)>(after_activity,after_user) ORDER BY activity_id,user_id LIMIT 1000) k;
  END IF;
  EXIT WHEN batch IS NULL;
  PERFORM r.activity_id FROM apub_reposts r WHERE activity_id IN (SELECT k.activity_id FROM jsonb_to_recordset(batch) AS k(activity_id text,user_id text)) ORDER BY activity_id FOR SHARE;
  UPDATE apub_repost_recipients r SET post_id=p.post_id,private_counted=p.visibility NOT IN ('public','local') AND p.user_id IS DISTINCT FROM r.user_id
   FROM apub_reposts p,jsonb_to_recordset(batch) AS k(activity_id text,user_id text) WHERE r.activity_id=k.activity_id AND r.user_id=k.user_id AND p.activity_id=r.activity_id;
  after_activity:=batch->-1->>'activity_id'; after_user:=batch->-1->>'user_id';
  COMMIT;
 END LOOP;
END $$;
CREATE TRIGGER home_private_repost_totals_change AFTER INSERT OR UPDATE OR DELETE ON apub_repost_recipients FOR EACH ROW EXECUTE FUNCTION maintain_home_repost_totals();

INSERT INTO home_reaction_totals(post_id,source_kind,group_key,reaction,emoji)
 SELECT DISTINCT post_id,kind,home_reaction_key(reaction,emoji),reaction,emoji FROM (
  SELECT post_id,CASE WHEN user_id IS NULL THEN 0 ELSE 1 END::smallint AS kind,reaction,emoji FROM apub_reactions
  UNION ALL SELECT message_id,2::smallint,emoji,NULL::jsonb FROM message_reactions
 ) r ORDER BY post_id,kind,home_reaction_key(reaction,emoji),reaction,emoji ON CONFLICT DO NOTHING;
INSERT INTO home_repost_totals(post_id) SELECT DISTINCT post_id FROM apub_reposts WHERE visibility IN ('public','local') ORDER BY post_id ON CONFLICT DO NOTHING;
INSERT INTO home_private_repost_totals(user_id,post_id) SELECT DISTINCT user_id,post_id FROM apub_repost_recipients WHERE private_counted ORDER BY user_id,post_id ON CONFLICT DO NOTHING;

-- Lock before taking each source-count snapshot: a concurrent writer cannot
-- commit its source row while its corresponding delta waits for this lock.
DO $$
DECLARE after_post text; after_kind smallint; after_key text; batch jsonb; item record;
BEGIN
 LOOP
  IF after_post IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT post_id,source_kind,group_key FROM home_reaction_totals ORDER BY post_id,source_kind,group_key LIMIT 100) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT post_id,source_kind,group_key FROM home_reaction_totals WHERE (post_id,source_kind,group_key)>(after_post,after_kind,after_key) ORDER BY post_id,source_kind,group_key LIMIT 100) k;
  END IF;
  EXIT WHEN batch IS NULL;
  FOR item IN SELECT * FROM jsonb_to_recordset(batch) AS k(post_id text,source_kind smallint,group_key text) ORDER BY post_id,source_kind,group_key LOOP
   PERFORM 1 FROM home_reaction_totals WHERE (post_id,source_kind,group_key)=(item.post_id,item.source_kind,item.group_key) FOR UPDATE;
   UPDATE home_reaction_totals c SET n=CASE WHEN item.source_kind=2 THEN
    (SELECT count(*) FROM message_reactions r WHERE r.message_id=item.post_id AND home_reaction_key(r.emoji,NULL::jsonb)=item.group_key)
    ELSE (SELECT count(*) FROM apub_reactions r WHERE r.post_id=item.post_id AND (r.user_id IS NULL)=(item.source_kind=0) AND home_reaction_key(r.reaction,r.emoji)=item.group_key) END
    WHERE (c.post_id,c.source_kind,c.group_key)=(item.post_id,item.source_kind,item.group_key);
   after_post:=item.post_id;after_kind:=item.source_kind;after_key:=item.group_key;
  END LOOP;
  COMMIT;
 END LOOP;
END $$;
DO $$
DECLARE after_post text; batch jsonb; item record;
BEGIN
 LOOP
  IF after_post IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT post_id FROM home_repost_totals ORDER BY post_id LIMIT 100) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT post_id FROM home_repost_totals WHERE post_id>after_post ORDER BY post_id LIMIT 100) k;
  END IF;
  EXIT WHEN batch IS NULL;
  FOR item IN SELECT * FROM jsonb_to_recordset(batch) AS k(post_id text) ORDER BY post_id LOOP
   PERFORM 1 FROM home_repost_totals WHERE post_id=item.post_id FOR UPDATE;
   UPDATE home_repost_totals c SET n=(SELECT count(*) FROM apub_reposts r WHERE r.post_id=item.post_id AND visibility IN ('public','local')) WHERE c.post_id=item.post_id;
   after_post:=item.post_id;
  END LOOP;
  COMMIT;
 END LOOP;
END $$;
DO $$
DECLARE after_user text; after_post text; batch jsonb; item record;
BEGIN
 LOOP
  IF after_user IS NULL THEN
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT user_id,post_id FROM home_private_repost_totals ORDER BY user_id,post_id LIMIT 100) k;
  ELSE
   SELECT jsonb_agg(to_jsonb(k)) INTO batch FROM (SELECT user_id,post_id FROM home_private_repost_totals WHERE (user_id,post_id)>(after_user,after_post) ORDER BY user_id,post_id LIMIT 100) k;
  END IF;
  EXIT WHEN batch IS NULL;
  FOR item IN SELECT * FROM jsonb_to_recordset(batch) AS k(user_id text,post_id text) ORDER BY user_id,post_id LOOP
   PERFORM 1 FROM home_private_repost_totals WHERE (user_id,post_id)=(item.user_id,item.post_id) FOR UPDATE;
   UPDATE home_private_repost_totals c SET n=(SELECT count(*) FROM apub_repost_recipients r WHERE r.user_id=item.user_id AND r.post_id=item.post_id AND r.private_counted) WHERE (c.user_id,c.post_id)=(item.user_id,item.post_id);
   after_user:=item.user_id;after_post:=item.post_id;
  END LOOP;
  COMMIT;
 END LOOP;
END $$;
ANALYZE home_reaction_totals;
ANALYZE home_repost_totals;
ANALYZE home_private_repost_totals;
