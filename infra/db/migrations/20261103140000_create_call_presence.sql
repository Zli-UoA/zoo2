-- Ephemeral call room presence across multi-replica backend instances.
CREATE TABLE call_presence (
  channel_id text NOT NULL,
  session_id uuid NOT NULL,
  user_id text NOT NULL,
  user_name text NOT NULL,
  display_name text NOT NULL,
  is_muted boolean NOT NULL DEFAULT false,
  joined_at text NOT NULL,
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (channel_id, session_id),
  CONSTRAINT call_presence_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT call_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_call_presence_channel ON call_presence (channel_id, expires_at);
CREATE INDEX idx_call_presence_expires_at ON call_presence (expires_at);
