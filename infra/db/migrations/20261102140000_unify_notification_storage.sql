
-- Canonical inbox. Legacy tables remain rolling-deployment projections only.
CREATE TABLE notification_events (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  source text NOT NULL DEFAULT 'channel' CHECK (source IN ('channel', 'activitypub')),
  actor_id text NOT NULL,
  channel_id text REFERENCES channels(id) ON UPDATE CASCADE ON DELETE CASCADE,
  message_id text REFERENCES messages(id) ON UPDATE CASCADE ON DELETE CASCADE,
  notification_type text NOT NULL,
  activity_id text,
  post_id text,
  reaction text,
  emoji jsonb,
  is_read boolean NOT NULL DEFAULT false,
  created_at timestamp(6) NOT NULL DEFAULT timezone('UTC', now()),
  CHECK ((source = 'channel' AND channel_id IS NOT NULL AND message_id IS NOT NULL AND activity_id IS NULL)
      OR (source = 'activitypub' AND channel_id IS NULL AND message_id IS NULL AND activity_id IS NOT NULL
          AND notification_type IN ('follow_request','follow_accepted','like','reply','repost'))),
  UNIQUE (user_id, activity_id, notification_type)
);
CREATE INDEX notification_events_owner ON notification_events(user_id, created_at DESC, id DESC);
CREATE INDEX notification_events_unread ON notification_events(user_id) WHERE NOT is_read;
CREATE INDEX notification_events_message ON notification_events(message_id) WHERE message_id IS NOT NULL;
ALTER TABLE notification_events REPLICA IDENTITY FULL;

-- Updatable, storage-free projections preserve the existing source-specific DTOs.
CREATE VIEW channel_notifications AS
  SELECT id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at
  FROM notification_events WHERE source = 'channel' WITH LOCAL CHECK OPTION;
CREATE VIEW federated_notifications AS
  SELECT id,user_id,actor_id,activity_id,notification_type AS kind,post_id,is_read,
         created_at AT TIME ZONE 'UTC' AS created_at,reaction,emoji
  FROM notification_events WHERE source = 'activitypub';

-- Preserve IDs, timestamps and read state. Lock both sources for an atomic handover.
-- Atlas replays statements without a surrounding transaction during validation.
-- Keep locking, backfill and compatibility-trigger installation atomic in both paths.
DO $inbox_migration$
BEGIN
LOCK TABLE notifications, apub_notifications IN SHARE ROW EXCLUSIVE MODE;
INSERT INTO notification_events(id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at)
  SELECT id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at FROM notifications;
INSERT INTO notification_events(id,user_id,source,actor_id,activity_id,notification_type,post_id,is_read,created_at,reaction,emoji)
  SELECT id,user_id,'activitypub',actor_id,activity_id,kind,post_id,is_read,created_at AT TIME ZONE 'UTC',reaction,emoji FROM apub_notifications;

CREATE FUNCTION sync_notification_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF TG_TABLE_NAME = 'notification_events' THEN
      IF OLD.source = 'channel' THEN DELETE FROM notifications WHERE id = OLD.id;
      ELSE DELETE FROM apub_notifications WHERE id = OLD.id; END IF;
    ELSE
      DELETE FROM notification_events WHERE id = OLD.id
        AND source = CASE WHEN TG_TABLE_NAME = 'notifications' THEN 'channel' ELSE 'activitypub' END;
    END IF;
    RETURN OLD;
  END IF;
  IF TG_TABLE_NAME = 'notifications' THEN
    INSERT INTO notification_events AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at,source) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at,'channel')
    ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,source=EXCLUDED.source
    WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at,current.source) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.source);
  ELSIF TG_TABLE_NAME = 'apub_notifications' THEN
    INSERT INTO notification_events AS current (id,user_id,actor_id,activity_id,notification_type,post_id,is_read,created_at,reaction,emoji,source) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.kind,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji,'activitypub')
    ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,notification_type=EXCLUDED.notification_type,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji,source=EXCLUDED.source
    WHERE (current.user_id,current.actor_id,current.activity_id,current.notification_type,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji,current.source) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.notification_type,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji,EXCLUDED.source);
  ELSE
    IF NEW.source = 'channel' THEN
      INSERT INTO notifications AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at
      WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at);
    ELSE
      INSERT INTO apub_notifications AS current (id,user_id,actor_id,activity_id,kind,post_id,is_read,created_at,reaction,emoji) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.notification_type,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,kind=EXCLUDED.kind,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji
      WHERE (current.user_id,current.actor_id,current.activity_id,current.kind,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.kind,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji);
    END IF;
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER notifications_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON notifications
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();
CREATE TRIGGER apub_notifications_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON apub_notifications
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();
CREATE TRIGGER notification_events_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON notification_events
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();
END;
$inbox_migration$;
