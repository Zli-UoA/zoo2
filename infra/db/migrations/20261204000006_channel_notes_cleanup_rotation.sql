CREATE TABLE channel_notes_cleanup_cursor (
  id boolean PRIMARY KEY DEFAULT true CHECK (id),
  last_channel_id text
);

INSERT INTO channel_notes_cleanup_cursor (id, last_channel_id) VALUES (true, NULL);
