-- Drain message writers before locking the inbox and backfilling. This keeps
-- the migration's lock order consistent with soft deletion (message -> inbox).
DO $$ BEGIN LOCK TABLE messages IN SHARE MODE; END $$;

ALTER TABLE notification_events ADD COLUMN message_deleted boolean NOT NULL DEFAULT true;

-- Keep message visibility on the small inbox, rather than replicating every
-- undeleted message ID into Electric's notification dependency Shape.
CREATE FUNCTION set_notification_message_deleted() RETURNS trigger LANGUAGE plpgsql AS $$
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
CREATE TRIGGER notification_message_deleted BEFORE INSERT OR UPDATE OF source,message_id,message_deleted ON notification_events
  FOR EACH ROW EXECUTE FUNCTION set_notification_message_deleted();

CREATE FUNCTION sync_notification_message_deleted() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE notification_events SET message_deleted = (NEW.deleted_at IS NOT NULL)
    WHERE source = 'channel' AND message_id = NEW.id
      AND message_deleted IS DISTINCT FROM (NEW.deleted_at IS NOT NULL);
  RETURN NULL;
END;
$$;
CREATE TRIGGER message_notification_deleted AFTER UPDATE OF deleted_at ON messages
  FOR EACH ROW WHEN (OLD.deleted_at IS DISTINCT FROM NEW.deleted_at)
  EXECUTE FUNCTION sync_notification_message_deleted();

-- Backfill before the new backend starts. Legacy writers remain supported by
-- the BEFORE trigger throughout the rolling deployment.
UPDATE notification_events n SET message_deleted = (m.deleted_at IS NOT NULL)
  FROM messages m WHERE n.source = 'channel' AND n.message_id = m.id
    AND n.message_deleted IS DISTINCT FROM (m.deleted_at IS NOT NULL);
