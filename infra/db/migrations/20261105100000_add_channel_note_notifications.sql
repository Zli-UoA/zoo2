ALTER TABLE notifications ALTER COLUMN message_id DROP NOT NULL;
ALTER TABLE notifications ADD COLUMN note_id text REFERENCES channel_notes(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE notifications ADD CONSTRAINT notifications_source_check CHECK ((message_id IS NOT NULL) <> (note_id IS NOT NULL));
CREATE INDEX idx_notifications_note_id ON notifications (note_id);
