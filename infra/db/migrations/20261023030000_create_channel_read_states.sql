CREATE TABLE channel_read_states (
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  channel_id text NOT NULL REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE,
  last_read_at timestamp(3) NOT NULL,
  PRIMARY KEY (user_id, channel_id)
);

CREATE INDEX idx_messages_channel_unread ON messages (channel_id, created_at)
  WHERE parent_id IS NULL AND deleted_at IS NULL;
