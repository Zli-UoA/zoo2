-- Per-user pinned channels and DMs, ordered independently for each user.
CREATE TABLE user_pinned_channels (
  user_id text NOT NULL REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  channel_id text NOT NULL REFERENCES channels(id) ON UPDATE CASCADE ON DELETE CASCADE,
  position integer NOT NULL CHECK (position >= 0),
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (user_id, channel_id)
);
CREATE INDEX idx_user_pinned_channels_user_position ON user_pinned_channels (user_id, position, channel_id);
-- チャンネル削除時に、そのチャンネルのピンを全ユーザー分たどる。
CREATE INDEX idx_user_pinned_channels_channel ON user_pinned_channels (channel_id);

-- Version counter serialized per user. Every pin mutation locks this row.
CREATE TABLE user_channel_pin_states (
  user_id text NOT NULL PRIMARY KEY REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
  updated_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- ElectricSQL needs the full old row for UPDATE/DELETE replication.
ALTER TABLE user_pinned_channels REPLICA IDENTITY FULL;
