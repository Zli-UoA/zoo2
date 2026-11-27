ALTER TABLE home_images
  ADD COLUMN thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN thumbnail_ready boolean NOT NULL DEFAULT false;

CREATE OR REPLACE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletions(id,uploader_id,file_size)
  VALUES(OLD.id,OLD.uploader_id,OLD.file_size + OLD.thumbnail_bytes) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;
