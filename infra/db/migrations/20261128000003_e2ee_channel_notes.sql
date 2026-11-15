-- atlas:txmode file

-- Keep plaintext channel notes out of encrypted channels and do not strand notes
-- that can still be restored when a channel is encrypted.

-- Do not silently leave restorable plaintext notes in channels that were
-- encrypted before this trigger existed. Without the clients' E2EE keys there
-- is no safe automatic way to migrate or redact this content. Abort deployment
-- while retaining the rows so they can be resolved without data loss.
-- Hold both tables against writes from before the preflight through trigger
-- creation. Older note writers can hold channel_notes before reaching channels
-- through its FK, so waiting for the second lock while retaining the first can
-- deadlock. Fail fast instead; Atlas leaves this file unapplied, rolls the
-- transaction back, and a later deployment can retry after writers drain.
-- The preflight and guards remain atomic in the same migration transaction.
LOCK TABLE channels IN SHARE ROW EXCLUSIVE MODE NOWAIT;
LOCK TABLE channel_notes IN SHARE ROW EXCLUSIVE MODE NOWAIT;

DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM channel_notes n
    JOIN channels c ON c.id = n.channel_id
    WHERE c.encrypted
      AND (n.deleted_at IS NULL OR n.deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours')
  ) THEN
    RAISE EXCEPTION 'cannot install E2EE channel note protection while encrypted channels contain active or restorable notes'
      USING ERRCODE = '55000';
  END IF;
END $$;

-- Keep the branch's encryption-immutable function and, after main is merged,
-- its conversion function. A notes-only trigger preserves both sets of checks.
CREATE FUNCTION e2ee_reject_channel_encryption_with_notes() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NOT OLD.encrypted AND NEW.encrypted AND EXISTS (
    SELECT 1 FROM channel_notes
    WHERE channel_id = NEW.id
      AND (deleted_at IS NULL OR deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours')
  ) THEN
    RAISE EXCEPTION 'remove active and restorable channel notes before encrypting the channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channels_encrypted_reject_notes BEFORE UPDATE OF encrypted ON channels
  FOR EACH ROW WHEN (OLD.encrypted IS DISTINCT FROM NEW.encrypted AND NEW.encrypted)
  EXECUTE FUNCTION e2ee_reject_channel_encryption_with_notes();

CREATE FUNCTION e2ee_reject_plaintext_channel_notes() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE enc boolean;
BEGIN
  SELECT encrypted INTO enc FROM channels WHERE id = NEW.channel_id FOR SHARE;
  IF COALESCE(enc, false) THEN
    RAISE EXCEPTION 'channel notes are unavailable in encrypted channels' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channel_notes_e2ee_reject_plaintext BEFORE INSERT OR UPDATE ON channel_notes
  FOR EACH ROW EXECUTE FUNCTION e2ee_reject_plaintext_channel_notes();
