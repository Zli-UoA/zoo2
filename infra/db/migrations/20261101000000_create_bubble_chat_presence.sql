-- Shared ephemeral bubble chat state; the database clock is authoritative for expiry.
CREATE TABLE bubble_chat_presence (
  user_id text NOT NULL,
  session_id text NOT NULL,
  content text NOT NULL,
  seq bigint NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  retention_until timestamptz NOT NULL,
  PRIMARY KEY (user_id, session_id),
  CONSTRAINT bubble_chat_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_bubble_chat_presence_expires_at ON bubble_chat_presence (expires_at);
CREATE INDEX idx_bubble_chat_presence_retention_until ON bubble_chat_presence (retention_until);
