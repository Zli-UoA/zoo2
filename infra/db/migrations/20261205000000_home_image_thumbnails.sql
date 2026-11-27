ALTER TABLE home_images
  ADD COLUMN thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN thumbnail_ready boolean,
  ADD COLUMN thumbnail_conditional_write boolean NOT NULL DEFAULT false,
  ADD COLUMN thumbnail_check_at timestamptz NOT NULL DEFAULT (now()+interval '5 minutes');

ALTER TABLE home_image_deletions
  ADD COLUMN thumbnail_accounted boolean NOT NULL DEFAULT false,
  ADD COLUMN unconfirmed_thumbnail_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN legacy_write_pending boolean NOT NULL DEFAULT true;

CREATE FUNCTION home_image_can_have_thumbnail(content_type text, width integer, height integer)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT content_type IN ('image/png','image/jpeg','image/webp')
    AND (width IS NULL OR height IS NULL OR GREATEST(width,height)>768)
$$;

CREATE OR REPLACE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletions(id,uploader_id,file_size,thumbnail_accounted,unconfirmed_thumbnail_bytes,legacy_write_pending)
  VALUES(OLD.id,OLD.uploader_id,
    OLD.file_size + CASE WHEN OLD.thumbnail_ready THEN OLD.thumbnail_bytes ELSE 0 END,
    true,
    CASE WHEN OLD.thumbnail_ready IS NULL THEN
      CASE WHEN OLD.thumbnail_bytes>0 THEN OLD.thumbnail_bytes
           WHEN home_image_can_have_thumbnail(OLD.content_type,OLD.width,OLD.height) THEN OLD.file_size
           ELSE 0 END
      ELSE 0 END,
    NOT OLD.thumbnail_conditional_write) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;

-- Older workers may already have deleted the object, but must leave the queue row
-- so a subsequent pass can remove any late storage write.
CREATE FUNCTION protect_pending_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.legacy_write_pending AND current_setting('zoo.home_image_legacy_writes_drained', true) IS DISTINCT FROM 'on' THEN
    RETURN NULL;
  END IF;
  RETURN OLD;
END $$;
CREATE TRIGGER home_image_deletion_pending BEFORE DELETE ON home_image_deletions
  FOR EACH ROW EXECUTE FUNCTION protect_pending_home_image_deletion();

CREATE TABLE apub_media_blurhash_queue (
  post_id text PRIMARY KEY REFERENCES apub_remote_posts(id) ON DELETE CASCADE,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_id text,
  image_retry_counts jsonb
);
CREATE INDEX apub_media_blurhash_queue_available ON apub_media_blurhash_queue(available_at,post_id);
