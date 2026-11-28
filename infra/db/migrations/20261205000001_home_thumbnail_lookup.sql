-- atlas:txmode none

CREATE INDEX CONCURRENTLY home_image_deletions_legacy_gc_created ON home_image_deletion_queue
  ((CASE WHEN legacy_write_pending OR original_write_pending OR thumbnail_write_pending
            THEN 'infinity'::timestamptz ELSE created_at END));

-- The legacy application uses one relation for quota and row-locked cleanup.
-- The exact quota SELECT can include pending rows. Cleanup scans only finite dates,
-- using the same indexed expression for its upper bound and ORDER BY.
CREATE OR REPLACE VIEW home_image_deletions AS
SELECT id,uploader_id,file_size,
       CASE WHEN legacy_write_pending OR original_write_pending OR thumbnail_write_pending
            THEN 'infinity'::timestamptz ELSE created_at END AS created_at
FROM home_image_deletion_queue
WHERE (CASE WHEN legacy_write_pending OR original_write_pending OR thumbnail_write_pending
            THEN 'infinity'::timestamptz ELSE created_at END) <= (SELECT CASE
        WHEN current_query() = $legacy_quota$SELECT json_build_object('bytes',(SELECT COALESCE(sum(file_size),0) FROM home_images WHERE uploader_id=$1)+(SELECT COALESCE(sum(file_size),0) FROM home_image_deletions WHERE uploader_id=$1))::text$legacy_quota$
        THEN 'infinity'::timestamptz ELSE TIMESTAMPTZ '294276-12-31 23:59:59.999999+00' END);

CREATE INDEX CONCURRENTLY home_images_unconfirmed_thumbnail ON home_images(thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL;

CREATE INDEX CONCURRENTLY home_images_uploader_unconfirmed_thumbnail
  ON home_images(uploader_id,(thumbnail_bytes>0),thumbnail_check_at,id)
  WHERE thumbnail_ready IS NULL
    AND home_image_can_have_thumbnail(content_type,width,height);

CREATE INDEX CONCURRENTLY home_image_deletions_current_created ON home_image_deletion_queue(created_at)
  WHERE NOT legacy_write_pending;
