ALTER TABLE home_images
  ADD COLUMN thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN thumbnail_ready boolean,
  ADD COLUMN thumbnail_conditional_write boolean NOT NULL DEFAULT false,
  ADD COLUMN original_write_confirmed boolean NOT NULL DEFAULT false,
  ADD COLUMN thumbnail_write_pending boolean NOT NULL DEFAULT false,
  ADD COLUMN thumbnail_check_at timestamptz NOT NULL DEFAULT (now()+interval '5 minutes');

ALTER TABLE home_image_deletions RENAME TO home_image_deletion_queue;
ALTER TABLE home_image_deletion_queue
  ADD COLUMN thumbnail_accounted boolean NOT NULL DEFAULT false,
  ADD COLUMN unconfirmed_thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN legacy_write_pending boolean NOT NULL DEFAULT true,
  ADD COLUMN original_write_pending boolean NOT NULL DEFAULT false,
  ADD COLUMN thumbnail_write_pending boolean NOT NULL DEFAULT false;

CREATE FUNCTION home_image_can_have_thumbnail(content_type text, width integer, height integer)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT content_type IN ('image/png','image/jpeg','image/webp')
    AND (width IS NULL OR height IS NULL OR GREATEST(width,height)>768)
$$;

CREATE OR REPLACE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletion_queue(id,uploader_id,file_size,thumbnail_accounted,unconfirmed_thumbnail_bytes,legacy_write_pending,original_write_pending,thumbnail_write_pending)
  VALUES(OLD.id,OLD.uploader_id,
    OLD.file_size + CASE WHEN OLD.thumbnail_ready THEN OLD.thumbnail_bytes ELSE 0 END,
    true,
    CASE WHEN OLD.thumbnail_ready IS NULL THEN
      CASE WHEN OLD.thumbnail_bytes>0 THEN OLD.thumbnail_bytes
           WHEN home_image_can_have_thumbnail(OLD.content_type,OLD.width,OLD.height) THEN OLD.file_size
           ELSE 0 END
      ELSE 0 END,
    NOT OLD.thumbnail_conditional_write,
    OLD.thumbnail_conditional_write AND NOT OLD.original_write_confirmed,
    OLD.thumbnail_write_pending) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;

-- Keep the original ordering until the indexed ordering is available.
CREATE VIEW home_image_deletions AS
SELECT id,uploader_id,file_size,created_at
FROM home_image_deletion_queue
WHERE (NOT legacy_write_pending AND NOT original_write_pending AND NOT thumbnail_write_pending)
   OR (SELECT current_query() = $legacy_quota$SELECT json_build_object('bytes',(SELECT COALESCE(sum(file_size),0) FROM home_images WHERE uploader_id=$1)+(SELECT COALESCE(sum(file_size),0) FROM home_image_deletions WHERE uploader_id=$1))::text$legacy_quota$);

CREATE TABLE apub_media_blurhash_queue (
  post_id text PRIMARY KEY REFERENCES apub_remote_posts(id) ON DELETE CASCADE,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_id text,
  image_retry_counts jsonb
);
CREATE INDEX apub_media_blurhash_queue_available ON apub_media_blurhash_queue(available_at,post_id);
