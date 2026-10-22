-- Modify "channels" table
ALTER TABLE "channels" ADD COLUMN "is_dm" boolean NOT NULL DEFAULT false;
-- Create index "idx_channels_is_dm" to table: "channels"
CREATE INDEX "idx_channels_is_dm" ON "channels" ("is_dm");
