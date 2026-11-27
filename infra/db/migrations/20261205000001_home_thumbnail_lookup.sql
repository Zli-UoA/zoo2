-- atlas:txmode none

CREATE INDEX CONCURRENTLY home_images_unconfirmed_thumbnail ON home_images(thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL;

CREATE INDEX CONCURRENTLY home_images_uploader_unconfirmed_thumbnail
  ON home_images(uploader_id,(thumbnail_bytes>0),thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL
    AND home_image_can_have_thumbnail(content_type,width,height);

CREATE INDEX CONCURRENTLY home_image_deletions_current_created ON home_image_deletion_queue(created_at)
  WHERE NOT legacy_write_pending;
