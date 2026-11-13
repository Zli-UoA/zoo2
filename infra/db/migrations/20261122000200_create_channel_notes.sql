-- Create "channel_notes" table
CREATE TABLE "channel_notes" (
  "id" text NOT NULL,
  "channel_id" text NOT NULL,
  "title" character varying(200) NOT NULL,
  "content" text NOT NULL,
  "position" integer NOT NULL DEFAULT 0,
  "deleted_at" timestamp(3) NULL,
  "version" integer NOT NULL DEFAULT 1,
  "created_user_id" text NOT NULL,
  "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  "updated_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("id"),
  CONSTRAINT "channel_notes_channel_id_fkey" FOREIGN KEY ("channel_id") REFERENCES "channels" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT "channel_notes_created_user_id_fkey" FOREIGN KEY ("created_user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT "channel_notes_version_check" CHECK (version > 0)
);
-- Create index "idx_channel_notes_channel_position" to table: "channel_notes"
CREATE INDEX "idx_channel_notes_channel_position" ON "channel_notes" ("channel_id", "position") WHERE (deleted_at IS NULL);
-- Create index "idx_channel_notes_channel_updated" to table: "channel_notes"
CREATE INDEX "idx_channel_notes_channel_updated" ON "channel_notes" ("channel_id", "updated_at" DESC, "id");
-- Create index "idx_channel_notes_deleted_at" to table: "channel_notes"
CREATE INDEX "idx_channel_notes_deleted_at" ON "channel_notes" ("channel_id", "deleted_at") WHERE (deleted_at IS NOT NULL);
