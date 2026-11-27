-- Note writes are coordinated per channel by the API and cooperating SQL
-- callers. A statement trigger cannot safely infer every affected channel for
-- arbitrary SQL, and a global lock would unnecessarily serialize unrelated
-- channels. Remove the previous global lock trigger if it was installed.
DROP TRIGGER IF EXISTS channel_notes_serialize_writes ON channel_notes;
DROP FUNCTION IF EXISTS serialize_channel_note_writes();
