-- Create index "idx_messages_channel_id_created_at" to table: "messages"
CREATE INDEX "idx_messages_channel_id_created_at" ON "messages" ("channel_id", "created_at" DESC);
