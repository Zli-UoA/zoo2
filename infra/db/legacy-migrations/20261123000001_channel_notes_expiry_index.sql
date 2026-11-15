-- Create index "idx_channel_notes_expired" to table: "channel_notes"
CREATE INDEX "idx_channel_notes_expired" ON "channel_notes" ("deleted_at", "id") WHERE (deleted_at IS NOT NULL);
