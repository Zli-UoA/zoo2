CREATE TABLE channel_webhooks (
  id text NOT NULL PRIMARY KEY,
  channel_id text NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
  name varchar(80) NOT NULL,
  secret text NOT NULL,
  user_id text NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_channel_webhooks_channel_id ON channel_webhooks (channel_id);
