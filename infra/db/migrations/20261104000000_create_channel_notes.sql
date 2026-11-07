CREATE TABLE channel_notes (
  id text NOT NULL PRIMARY KEY,
  channel_id text NOT NULL REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE,
  title varchar(200) NOT NULL,
  content text NOT NULL,
  version integer NOT NULL DEFAULT 1 CHECK (version > 0),
  created_user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE RESTRICT,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_channel_notes_channel_updated ON channel_notes (channel_id, updated_at DESC, id);
