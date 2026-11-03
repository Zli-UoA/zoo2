-- Modify "apub_remote_posts" table
ALTER TABLE "apub_remote_posts" ADD COLUMN "media" jsonb NOT NULL DEFAULT '[]';
-- Modify "home_posts" table
ALTER TABLE "home_posts" ADD COLUMN "media" jsonb NOT NULL DEFAULT '[]';
-- Create "home_images" table
CREATE TABLE "home_images" (
  "id" text NOT NULL,
  "uploader_id" text NOT NULL,
  "content_type" text NOT NULL,
  "name" text NOT NULL,
  "file_size" bigint NOT NULL DEFAULT 0,
  PRIMARY KEY ("id"),
  CONSTRAINT "home_images_uploader_id_fkey" FOREIGN KEY ("uploader_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
