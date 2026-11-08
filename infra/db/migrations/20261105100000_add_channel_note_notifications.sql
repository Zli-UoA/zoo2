ALTER TABLE notification_events ADD COLUMN note_id text REFERENCES channel_notes(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE notification_events DROP CONSTRAINT notification_events_check;
ALTER TABLE notification_events ADD CONSTRAINT notification_events_check CHECK (
  (source = 'channel' AND channel_id IS NOT NULL AND ((message_id IS NOT NULL) <> (note_id IS NOT NULL)) AND activity_id IS NULL)
  OR (source = 'activitypub' AND channel_id IS NULL AND message_id IS NULL AND note_id IS NULL AND activity_id IS NOT NULL
      AND notification_type IN ('follow_request','follow_accepted','like','reply','repost'))
);
CREATE INDEX notification_events_note ON notification_events(note_id) WHERE note_id IS NOT NULL;
CREATE OR REPLACE VIEW channel_notifications AS
  SELECT id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at,note_id
  FROM notification_events WHERE source = 'channel' WITH LOCAL CHECK OPTION;
CREATE OR REPLACE FUNCTION sync_notification_events() RETURNS trigger LANGUAGE plpgsql AS $$
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
    IF NEW.source = 'channel' AND NEW.note_id IS NULL THEN
      INSERT INTO notifications AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at
      WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at);
    ELSIF NEW.source = 'activitypub' THEN
      INSERT INTO apub_notifications AS current (id,user_id,actor_id,activity_id,kind,post_id,is_read,created_at,reaction,emoji) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.notification_type,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,kind=EXCLUDED.kind,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji
      WHERE (current.user_id,current.actor_id,current.activity_id,current.kind,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.kind,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji);
    END IF;
  END IF;
  RETURN NEW;
END;
$$;
