BEGIN;
SET LOCAL jit=off;
-- No application-supplied display_data: old writers remain supported.
INSERT INTO users(id,user_name,display_name,is_bot) VALUES
 ('view-a','view-a','Alice',false),('view-b','view-b','Bob',false);
INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES ('view-c','view-c','view-a',false);
INSERT INTO workspace_emojis(name,url,created_by,aliases,width,height)
 VALUES ('view-wide','/api/emoji-images/wide','view-a','["view-alias"]',120,20);
INSERT INTO messages(id,user_id,channel_id,content) VALUES ('view-m','view-a','view-c',':view-alias: hello');
INSERT INTO notifications(id,user_id,actor_id,channel_id,message_id,notification_type)
 VALUES ('view-n','view-a','view-b','view-c','view-m','reaction');
INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES ('view-r','view-m','view-b',':view-wide:');
INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES ('view-reply','view-b','view-c','reply','view-m');
INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key)
 VALUES ('view-file','view-m','view-a','view-c','photo.png',100,'image/png','view-object');
DO $$ BEGIN
 IF (SELECT display_data->'author'->>'display_name' FROM messages WHERE id='view-m') IS DISTINCT FROM 'Alice' THEN RAISE EXCEPTION 'author'; END IF;
 IF (SELECT display_data->>'reply_count' FROM messages WHERE id='view-m') IS DISTINCT FROM '1' THEN RAISE EXCEPTION 'reply count'; END IF;
 IF (SELECT display_data->'emojis'->'view-alias'->>'width' FROM messages WHERE id='view-m') IS DISTINCT FROM '120' THEN RAISE EXCEPTION 'emoji alias'; END IF;
 IF (SELECT display_data->'message'->'emojis'->'view-wide'->>'height' FROM notification_events WHERE id='view-n') IS DISTINCT FROM '20' THEN RAISE EXCEPTION 'notification emoji'; END IF;
 IF (SELECT display_data->'attachments'->0->>'id' FROM messages WHERE id='view-m') IS DISTINCT FROM 'view-file' THEN RAISE EXCEPTION 'attachment'; END IF;
 IF EXISTS(SELECT 1 FROM messages m JOIN message_display v ON v.id=m.id WHERE m.channel_id='view-c' AND m.display_data IS DISTINCT FROM v.display_data) THEN RAISE EXCEPTION 'projection drift'; END IF;
END $$;
-- Adding/updating an unused emoji must not enqueue every colon-containing row.
INSERT INTO workspace_emojis(name,url,created_by) VALUES ('view-unused','/unused.png','view-a');
UPDATE workspace_emojis SET width=100,height=20 WHERE name='view-unused';
DELETE FROM workspace_emojis WHERE name='view-unused';
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM display_refresh_queue WHERE target IN ('view-m','view-reply')) THEN RAISE EXCEPTION 'unrelated emoji fan-out'; END IF;
END $$;
-- A notification contains only its actor's badge, and unrelated reactions do
-- not rewrite its cached row (or replicate it through Electric).
CREATE TEMP TABLE notification_before AS SELECT ctid AS row_version FROM notification_events WHERE id='view-n';
INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES ('view-other','view-m','view-a','👍');
DO $$ BEGIN
 IF (SELECT jsonb_array_length(display_data->'reactions') FROM notification_events WHERE id='view-n') <> 1 THEN RAISE EXCEPTION 'notification copied other actors'; END IF;
 IF (SELECT display_data->'reactions'->0->>'user_id' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'view-b' THEN RAISE EXCEPTION 'notification badge actor'; END IF;
 IF (SELECT ctid FROM notification_events WHERE id='view-n') IS DISTINCT FROM (SELECT row_version FROM notification_before) THEN RAISE EXCEPTION 'unrelated reaction rewrote notification'; END IF;
END $$;
UPDATE users SET display_name='Updated' WHERE id='view-b';
UPDATE channels SET channel_name='Renamed' WHERE id='view-c';
UPDATE messages SET deleted_at=now() WHERE id='view-reply';
UPDATE workspace_emojis SET width=240 WHERE name='view-wide';
UPDATE messages SET content='Edited :view-alias:' WHERE id='view-m';
DO $$ BEGIN
 IF (SELECT display_data->'message'->>'content' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'Edited :view-alias:' THEN RAISE EXCEPTION 'message edit must refresh without another related write'; END IF;
END $$;
DELETE FROM attachments WHERE id='view-file';
DO $$ BEGIN WHILE refresh_next_display() LOOP END LOOP; END $$;
DO $$ BEGIN
 IF (SELECT display_data->>'reply_count' FROM messages WHERE id='view-m') IS DISTINCT FROM '0' THEN RAISE EXCEPTION 'deleted reply'; END IF;
 IF (SELECT display_data->'reactions'->0->>'display_name' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'Updated' THEN RAISE EXCEPTION 'reaction user update'; END IF;
 IF (SELECT display_data->'message'->>'content' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'Edited :view-alias:' THEN RAISE EXCEPTION 'message edit'; END IF;
 IF (SELECT display_data->'channel'->>'channel_name' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'Renamed' THEN RAISE EXCEPTION 'channel rename'; END IF;
 IF (SELECT display_data->'emojis'->'view-alias'->>'width' FROM messages WHERE id='view-m') IS DISTINCT FROM '240' THEN RAISE EXCEPTION 'dimension update'; END IF;
 IF (SELECT display_data->'attachments' FROM messages WHERE id='view-m') IS DISTINCT FROM '[]'::jsonb THEN RAISE EXCEPTION 'attachment removal'; END IF;
END $$;
DELETE FROM message_reactions WHERE id='view-r';
-- Federated actors are joined too; no later HTTP overlay is needed to paint chips.
INSERT INTO home_posts(id,user_id,channel_id,content) VALUES ('view-post','view-a','view-c','Linked post');
INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key)
 VALUES ('https://view-peer.example/actor','https://view-peer.example/inbox','https://view-peer.example/followers','peer','Peer','https://view-peer.example/key','test');
INSERT INTO apub_reactions(post_id,actor_id,reaction,activity_id,activity_json,emoji)
 VALUES ('view-post','https://view-peer.example/actor',':cat:','https://view-peer.example/like','{}',
 jsonb_build_object('name',':cat:','image_id',repeat('a',64),'url','https://view-peer.example/cat.png','width',60,'height',20));
DO $$ BEGIN
 IF (SELECT display_data->'reactions'->0->>'user_name' FROM messages WHERE id='view-post') IS DISTINCT FROM 'peer@view-peer.example' THEN RAISE EXCEPTION 'remote actor join'; END IF;
 IF (SELECT display_data->'emojis'->('apub:'||repeat('a',64)||'::cat:')->>'width' FROM messages WHERE id='view-post') IS DISTINCT FROM '60' THEN RAISE EXCEPTION 'remote emoji dimensions'; END IF;
 IF NOT EXISTS(SELECT 1 FROM apub_emoji_images WHERE id=repeat('a',64)) THEN RAISE EXCEPTION 'emoji proxy not registered'; END IF;
END $$;
UPDATE apub_actors SET blocked=true WHERE id='https://view-peer.example/actor';
DO $$ BEGIN WHILE refresh_next_display() LOOP END LOOP; END $$;
DO $$ BEGIN
 IF (SELECT display_data->'reactions' FROM messages WHERE id='view-post') IS DISTINCT FROM '[]'::jsonb THEN RAISE EXCEPTION 'blocked actor'; END IF;
END $$;
UPDATE apub_actors SET blocked=false WHERE id='https://view-peer.example/actor';
UPDATE workspace_emojis SET image_id=repeat('a',64) WHERE name='view-wide';
DO $$ BEGIN WHILE refresh_next_display() LOOP END LOOP; END $$;
DO $$ BEGIN
 IF (SELECT display_data->'reactions'->0->>'emoji' FROM messages WHERE id='view-post') IS DISTINCT FROM ':view-wide:' THEN RAISE EXCEPTION 'local emoji identity'; END IF;
END $$;
DELETE FROM apub_reactions WHERE post_id='view-post';
DO $$ BEGIN
 IF (SELECT display_data->'reactions' FROM messages WHERE id='view-post') IS DISTINCT FROM '[]'::jsonb THEN RAISE EXCEPTION 'removed remote reaction'; END IF;
END $$;
DELETE FROM workspace_emojis WHERE name='view-wide';
UPDATE messages SET deleted_at=now() WHERE id='view-m';
DO $$ BEGIN
 IF (SELECT display_data->'message' FROM notification_events WHERE id='view-n') IS DISTINCT FROM 'null'::jsonb THEN RAISE EXCEPTION 'removed message leaked'; END IF;
 IF (SELECT display_data->'reactions' FROM notification_events WHERE id='view-n') IS DISTINCT FROM '[]'::jsonb THEN RAISE EXCEPTION 'removed reactions leaked'; END IF;
 IF (SELECT display_data->'emojis' FROM messages WHERE id='view-m') IS DISTINCT FROM '{}'::jsonb THEN RAISE EXCEPTION 'deleted emoji'; END IF;
END $$;
ROLLBACK;
