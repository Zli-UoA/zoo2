-- atlas:txmode none

CREATE INDEX CONCURRENTLY home_images_unconfirmed_thumbnail ON home_images(thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL;

CREATE INDEX CONCURRENTLY home_images_uploader_unconfirmed_thumbnail
  ON home_images(uploader_id,(thumbnail_bytes>0),thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL
    AND home_image_can_have_thumbnail(content_type,width,height);
