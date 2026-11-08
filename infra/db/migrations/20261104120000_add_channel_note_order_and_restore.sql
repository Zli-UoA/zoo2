ALTER TABLE channel_notes ADD COLUMN position integer NOT NULL DEFAULT 0;
ALTER TABLE channel_notes ADD COLUMN deleted_at timestamp(3);
WITH ordered AS (
  SELECT id, row_number() OVER (PARTITION BY channel_id ORDER BY updated_at, id) - 1 AS position
  FROM channel_notes
)
UPDATE channel_notes AS notes SET position = ordered.position FROM ordered WHERE notes.id = ordered.id;
CREATE INDEX idx_channel_notes_channel_position ON channel_notes (channel_id, position) WHERE deleted_at IS NULL;
CREATE INDEX idx_channel_notes_deleted_at ON channel_notes (channel_id, deleted_at) WHERE deleted_at IS NOT NULL;
