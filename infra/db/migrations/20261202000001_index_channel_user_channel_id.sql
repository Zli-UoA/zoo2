-- atlas:txmode none

-- DM 転送元の参加者取得などで channel_id 先頭の参照に使う（ChannelUser_pkey は (user_id, channel_id)）。
CREATE INDEX CONCURRENTLY IF NOT EXISTS "idx_channel_user_channel_id" ON "channel_user" ("channel_id", "user_id");
