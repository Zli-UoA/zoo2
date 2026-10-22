-- Shared ephemeral typing state; the database clock is authoritative for expiry.
CREATE TABLE typing_presence (
  channel_id text NOT NULL,
  parent_id text,
  user_id text NOT NULL,
  session_id text NOT NULL,
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (channel_id, user_id, session_id),
  CONSTRAINT typing_presence_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT typing_presence_parent_id_fkey
    FOREIGN KEY (parent_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT typing_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_typing_presence_scope ON typing_presence (channel_id, parent_id, expires_at);
CREATE INDEX idx_typing_presence_expires_at ON typing_presence (expires_at);
