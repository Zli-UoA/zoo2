ALTER TABLE home_images
  ADD COLUMN thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN thumbnail_ready boolean,
  ADD COLUMN thumbnail_check_at timestamptz NOT NULL DEFAULT (now()+interval '5 minutes');

CREATE INDEX home_images_unconfirmed_thumbnail ON home_images(thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL;

ALTER TABLE home_image_deletions
  ADD COLUMN thumbnail_accounted boolean NOT NULL DEFAULT false;

CREATE OR REPLACE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletions(id,uploader_id,file_size,thumbnail_accounted)
  VALUES(OLD.id,OLD.uploader_id,OLD.file_size + CASE
    WHEN OLD.thumbnail_ready IS NULL AND OLD.thumbnail_bytes=0 THEN OLD.file_size
    ELSE OLD.thumbnail_bytes END,true) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;

CREATE TABLE apub_media_blurhash_queue (
  post_id text PRIMARY KEY REFERENCES apub_remote_posts(id) ON DELETE CASCADE,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_id text,
  attempts integer NOT NULL DEFAULT 0
);
CREATE INDEX apub_media_blurhash_queue_available ON apub_media_blurhash_queue(available_at,post_id);
