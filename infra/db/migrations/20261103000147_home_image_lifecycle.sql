-- Create index "attachments_storage_key" to table: "attachments"
CREATE INDEX "attachments_storage_key" ON "attachments" ("storage_key");
-- Modify "home_images" table
ALTER TABLE "home_images" ADD COLUMN "created_at" timestamptz NOT NULL DEFAULT now();
-- Create index "home_images_created" to table: "home_images"
CREATE INDEX "home_images_created" ON "home_images" ("created_at");
-- Create index "home_images_uploader_created" to table: "home_images"
CREATE INDEX "home_images_uploader_created" ON "home_images" ("uploader_id", "created_at");
-- Create index "home_posts_live_media" to table: "home_posts"
CREATE INDEX "home_posts_live_media" ON "home_posts" USING GIN ("media" jsonb_path_ops) WHERE (deleted_at IS NULL);
-- Create "home_image_deletions" table
CREATE TABLE "home_image_deletions" (
  "id" text NOT NULL,
  "uploader_id" text NOT NULL,
  "file_size" bigint NOT NULL DEFAULT 0,
  "created_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("id")
);
-- Create index "home_image_deletions_created" to table: "home_image_deletions"
CREATE INDEX "home_image_deletions_created" ON "home_image_deletions" ("created_at");
-- Create index "home_image_deletions_uploader" to table: "home_image_deletions"
CREATE INDEX "home_image_deletions_uploader" ON "home_image_deletions" ("uploader_id");
-- Create "home_image_upload_rates" table
CREATE TABLE "home_image_upload_rates" (
  "user_id" text NOT NULL,
  "window_start" timestamptz NOT NULL,
  "requests" integer NOT NULL DEFAULT 0,
  PRIMARY KEY ("user_id", "window_start"),
  CONSTRAINT "home_image_upload_rates_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);

CREATE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletions(id,uploader_id,file_size) VALUES(OLD.id,OLD.uploader_id,OLD.file_size) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;
CREATE TRIGGER home_image_deleted AFTER DELETE ON home_images FOR EACH ROW EXECUTE FUNCTION queue_home_image_deletion();

