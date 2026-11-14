-- Voice-call invitations (#call-invites): the call_invites table, and room for 'missed_call'
-- notifications, which belong to a channel but have no message.
-- Backward compatible: old pods keep working; they only never write missed_call rows.
-- Call invitations (ringing). A pending row is a live ring; the sweeper turns an expired one into 'missed'.
CREATE TABLE call_invites (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  channel_id text NOT NULL REFERENCES channels(id) ON UPDATE CASCADE ON DELETE CASCADE,
  inviter_id text NOT NULL REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  invitee_id text NOT NULL REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','declined','cancelled','missed')),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  responded_at timestamptz
);
CREATE UNIQUE INDEX call_invites_pending_unique ON call_invites (channel_id, invitee_id) WHERE status = 'pending';
CREATE INDEX call_invites_invitee_pending ON call_invites (invitee_id) WHERE status = 'pending';
CREATE INDEX call_invites_inviter_pending ON call_invites (inviter_id) WHERE status = 'pending';
CREATE INDEX call_invites_expiry_pending ON call_invites (expires_at) WHERE status = 'pending';
ALTER TABLE call_invites REPLICA IDENTITY FULL;

-- Relax the inbox CHECK so a channel notification may omit message_id when it is a missed call.
ALTER TABLE notification_events ADD CONSTRAINT notification_events_source_shape CHECK (
  (source = 'channel' AND channel_id IS NOT NULL AND activity_id IS NULL
    AND (message_id IS NOT NULL OR notification_type = 'missed_call'))
  OR (source = 'activitypub' AND channel_id IS NULL AND message_id IS NULL AND activity_id IS NOT NULL
    AND notification_type IN ('follow_request','follow_accepted','like','reply','repost'))) NOT VALID;
DO $$
DECLARE old_check text;
BEGIN
  SELECT conname INTO old_check FROM pg_constraint
   WHERE conrelid = 'notification_events'::regclass AND contype = 'c'
     AND conname <> 'notification_events_source_shape'
     AND pg_get_constraintdef(oid) LIKE '%follow_request%';
  IF old_check IS NOT NULL THEN
    EXECUTE format('ALTER TABLE notification_events DROP CONSTRAINT %I', old_check);
  END IF;
END $$;
ALTER TABLE notification_events VALIDATE CONSTRAINT notification_events_source_shape;

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
    IF NEW.source = 'channel' THEN
      IF NEW.message_id IS NOT NULL THEN -- missed_call has no message and no legacy row
        INSERT INTO notifications AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at)
        ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at
        WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at);
      END IF;
    ELSE
      INSERT INTO apub_notifications AS current (id,user_id,actor_id,activity_id,kind,post_id,is_read,created_at,reaction,emoji) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.notification_type,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,kind=EXCLUDED.kind,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji
      WHERE (current.user_id,current.actor_id,current.activity_id,current.kind,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.kind,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji);
    END IF;
  END IF;
  RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION set_notification_message_deleted() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  -- Legacy/canonical inbox bridges use INSERT ... ON CONFLICT even for read
  -- receipts. Reuse the current projection for an unchanged parent instead of
  -- taking message locks while the bridge already owns the notification row.
  IF TG_OP = 'UPDATE' THEN
    IF NEW.source = OLD.source AND NEW.message_id IS NOT DISTINCT FROM OLD.message_id
      AND NEW.message_deleted = OLD.message_deleted THEN RETURN NEW; END IF;
  ELSIF pg_trigger_depth() > 1 THEN
    SELECT message_deleted INTO NEW.message_deleted FROM notification_events
      WHERE id = NEW.id AND source = NEW.source
        AND message_id IS NOT DISTINCT FROM NEW.message_id;
    IF FOUND THEN RETURN NEW; END IF;
  END IF;
  IF NEW.source = 'channel' AND NEW.message_id IS NULL THEN
    NEW.message_deleted := false; -- missed_call has no message to be deleted
    RETURN NEW;
  END IF;
  NEW.message_deleted := true;
  IF NEW.source = 'channel' THEN
    -- Serialize with soft deletion/restoration. A plain SELECT can see the old
    -- message while its updater cannot yet see this new notification.
    SELECT deleted_at IS NOT NULL INTO NEW.message_deleted
      FROM messages WHERE id = NEW.message_id FOR SHARE;
    NEW.message_deleted := COALESCE(NEW.message_deleted, true);
  END IF;
  RETURN NEW;
END;
$$;
