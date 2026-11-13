BEGIN;
INSERT INTO users(id,user_name,display_name,is_bot) VALUES ('visibility-u','visibility-u','Visibility',false);
INSERT INTO channels(id,channel_name,created_user_id,is_private)
 VALUES ('visibility-c','visibility-c','visibility-u',false);
INSERT INTO messages(id,user_id,channel_id,content) VALUES
 ('visibility-active','visibility-u','visibility-c','Active'),
 ('visibility-deleted','visibility-u','visibility-c','Deleted');
UPDATE messages SET deleted_at=CURRENT_TIMESTAMP WHERE id='visibility-deleted';
-- Old writers use the legacy table and never supply the projection column.
INSERT INTO notifications(id,user_id,actor_id,channel_id,message_id,notification_type) VALUES
 ('visibility-n','visibility-u','visibility-u','visibility-c','visibility-active','mention'),
 ('visibility-d','visibility-u','visibility-u','visibility-c','visibility-deleted','mention');
DO $$ BEGIN
 IF (SELECT message_deleted FROM notification_events WHERE id='visibility-n') IS DISTINCT FROM false THEN RAISE EXCEPTION 'legacy active insert'; END IF;
 IF (SELECT message_deleted FROM notification_events WHERE id='visibility-d') IS DISTINCT FROM true THEN RAISE EXCEPTION 'legacy deleted insert'; END IF;
END $$;
-- Canonical/view writers, including explicit incorrect projection values.
INSERT INTO channel_notifications(id,user_id,actor_id,channel_id,message_id,notification_type)
 VALUES ('visibility-v','visibility-u','visibility-u','visibility-c','visibility-active','mention');
UPDATE notification_events SET message_deleted=true WHERE id='visibility-v';
DO $$ BEGIN
 IF (SELECT message_deleted FROM notification_events WHERE id='visibility-v') IS DISTINCT FROM false THEN RAISE EXCEPTION 'projection must follow parent'; END IF;
END $$;
UPDATE messages SET deleted_at=CURRENT_TIMESTAMP WHERE id='visibility-active';
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM notification_events WHERE user_id='visibility-u' AND NOT message_deleted) THEN RAISE EXCEPTION 'soft deletion was not atomic'; END IF;
END $$;
UPDATE messages SET deleted_at=NULL WHERE id='visibility-active';
DO $$ BEGIN
 IF (SELECT count(*) FROM notification_events WHERE user_id='visibility-u' AND NOT message_deleted) <> 2 THEN RAISE EXCEPTION 'restoration'; END IF;
END $$;
UPDATE channel_notifications SET message_id='visibility-deleted' WHERE id='visibility-v';
DO $$ BEGIN
 IF (SELECT message_deleted FROM notification_events WHERE id='visibility-v') IS DISTINCT FROM true THEN RAISE EXCEPTION 'reassignment'; END IF;
END $$;
-- ActivityPub inbox rows must neither read a message nor become channel-visible.
INSERT INTO notification_events(id,user_id,actor_id,source,activity_id,notification_type)
 VALUES ('visibility-ap','visibility-u','visibility-u','activitypub','visibility-activity','like');
DO $$ BEGIN
 IF (SELECT message_deleted FROM notification_events WHERE id='visibility-ap') IS DISTINCT FROM true THEN RAISE EXCEPTION 'activitypub isolation'; END IF;
END $$;
DELETE FROM messages WHERE id='visibility-active';
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM notification_events WHERE id='visibility-n') THEN RAISE EXCEPTION 'hard delete cascade'; END IF;
END $$;
ROLLBACK;
