-- Each ordered stream stops after a page of eligible keys. Payloads are fetched
-- only after merging those bounded streams; total post history is never sorted.
WITH visible AS NOT MATERIALIZED (
/* visible_posts */
), all_entries AS NOT MATERIALIZED (
/* timeline_entries */
), params AS (SELECT $8::text,$9::jsonb), cursor_ids AS (
  SELECT $2::text AS id WHERE $2<>''
  UNION
  SELECT post_id FROM apub_reposts WHERE activity_id=CASE WHEN starts_with($2,'repost:') THEN substr($2,8) END
), cursor AS MATERIALIZED (
  SELECT e.sort_at,e.entry_id FROM cursor_ids c CROSS JOIN LATERAL (
    SELECT sort_at,entry_id FROM all_entries WHERE id=c.id AND activity_id IS NULL AND entry_id=$2 OFFSET 0
  ) e
  UNION ALL
  SELECT e.sort_at,e.entry_id FROM apub_reposts r CROSS JOIN LATERAL (
    SELECT sort_at,entry_id FROM all_entries WHERE activity_id=r.activity_id AND id=r.post_id OFFSET 0
  ) e WHERE r.activity_id=CASE WHEN starts_with($2,'repost:') THEN substr($2,8) END
), followed AS MATERIALIZED (
  SELECT f.actor_id FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id AND NOT a.blocked
  WHERE f.user_id=$1 AND f.direction='outgoing' AND f.accepted AND ($7='' OR f.actor_id=$7)
), candidates AS (
  (SELECT e.id,e.entry_id,e.sort_at FROM home_posts p CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE id=p.id AND activity_id IS NULL OFFSET 0
   ) e WHERE $3 IN ('local','social') AND NOT $4::boolean AND p.deleted_at IS NULL AND p.visibility IN ('public','local') AND ($7='' OR p.user_id=$7)
     AND ($2='' OR (p.created_at,p.id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY p.created_at DESC,p.id DESC LIMIT $5::integer)
  UNION
  (SELECT e.id,e.entry_id,e.sort_at FROM home_posts p CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE id=p.id AND activity_id IS NULL OFFSET 0
   ) e WHERE ($3 IN ('home','social') OR $4::boolean AND $3='local') AND ($3<>'local' OR p.visibility IN ('public','local')) AND p.deleted_at IS NULL AND p.user_id=$1 AND ($7='' OR p.user_id=$7)
     AND ($2='' OR (p.created_at,p.id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY p.created_at DESC,p.id DESC LIMIT $5::integer)
  UNION
  (SELECT e.id,e.entry_id,e.sort_at FROM home_posts p CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE id=p.id AND activity_id IS NULL OFFSET 0
   ) e WHERE ($3 IN ('home','social') OR $4::boolean AND $3='local') AND ($3<>'local' OR p.visibility IN ('public','local')) AND p.deleted_at IS NULL AND p.reply_user_id=$1 AND p.user_id<>$1 AND ($7='' OR p.user_id=$7)
     AND ($2='' OR (p.created_at,p.id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY p.created_at DESC,p.id DESC LIMIT $5::integer)
  UNION
  SELECT e.id,e.entry_id,e.sort_at FROM channel_user membership CROSS JOIN LATERAL (
    SELECT eligible.id,eligible.entry_id,eligible.sort_at FROM home_posts p CROSS JOIN LATERAL (
      SELECT id,entry_id,sort_at FROM all_entries WHERE id=p.id AND activity_id IS NULL OFFSET 0
    ) eligible WHERE $4::boolean AND $3 IN ('local','social') AND membership.user_id=$1
      AND p.channel_id=membership.channel_id AND p.deleted_at IS NULL AND p.visibility IN ('public','local')
      AND ($7='' OR p.user_id=$7)
      AND ($2='' OR (p.created_at,p.id)<(SELECT sort_at,entry_id FROM cursor))
    ORDER BY p.created_at DESC,p.id DESC LIMIT $5::integer
  ) e WHERE membership.user_id=$1 AND $4::boolean
  UNION
  SELECT e.id,e.entry_id,e.sort_at FROM followed f CROSS JOIN LATERAL (
    SELECT eligible.id,eligible.entry_id,eligible.sort_at FROM apub_remote_recipients r CROSS JOIN LATERAL (
      SELECT id,entry_id,sort_at FROM all_entries WHERE id=r.post_id AND activity_id IS NULL OFFSET 0
    ) eligible WHERE $3 IN ('home','social') AND r.user_id=$1 AND r.actor_id=f.actor_id AND NOT r.post_deleted
      AND ($2='' OR (r.published,r.post_id)<(SELECT sort_at,entry_id FROM cursor))
    ORDER BY r.published DESC,r.post_id DESC LIMIT $5::integer
  ) e
  UNION
  (SELECT e.id,e.entry_id,e.sort_at FROM apub_remote_recipients r CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE id=r.post_id AND activity_id IS NULL OFFSET 0
   ) e WHERE $3 IN ('home','social') AND r.user_id=$1 AND r.reply_to_user AND NOT r.post_deleted AND ($7='' OR r.actor_id=$7)
     AND ($2='' OR (r.published,r.post_id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY r.published DESC,r.post_id DESC LIMIT $5::integer)
  UNION
  (SELECT e.id,e.entry_id,e.sort_at FROM apub_reposts r CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE activity_id=r.activity_id AND id=r.post_id OFFSET 0
   ) e WHERE $7='' AND $3 IN ('local','social') AND r.user_id IS NOT NULL AND r.visibility IN ('local','public')
     AND ($2='' OR (r.created_at,'repost:'||r.activity_id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY r.created_at DESC,('repost:'||r.activity_id) DESC LIMIT $5::integer)
  UNION
  (SELECT e.id,e.entry_id,e.sort_at FROM apub_reposts r CROSS JOIN LATERAL (
     SELECT id,entry_id,sort_at FROM all_entries WHERE activity_id=r.activity_id AND id=r.post_id OFFSET 0
   ) e WHERE $7='' AND $3 IN ('home','social') AND r.user_id=$1 AND ($3='home' OR r.visibility NOT IN ('local','public'))
     AND ($2='' OR (r.created_at,'repost:'||r.activity_id)<(SELECT sort_at,entry_id FROM cursor))
   ORDER BY r.created_at DESC,('repost:'||r.activity_id) DESC LIMIT $5::integer)
  UNION
  SELECT e.id,e.entry_id,e.sort_at FROM followed f CROSS JOIN LATERAL (
    SELECT eligible.id,eligible.entry_id,eligible.sort_at FROM apub_repost_recipients r CROSS JOIN LATERAL (
      SELECT id,entry_id,sort_at FROM all_entries WHERE activity_id=r.activity_id OFFSET 0
    ) eligible WHERE $7='' AND $3 IN ('home','social') AND r.user_id=$1 AND r.actor_id=f.actor_id
      AND ($2='' OR (r.created_at,'repost:'||r.activity_id)<(SELECT sort_at,entry_id FROM cursor))
    ORDER BY r.created_at DESC,('repost:'||r.activity_id) DESC LIMIT $5::integer
  ) e
)
SELECT json_build_object('entry_id',entry_id)::text FROM candidates
ORDER BY sort_at DESC,entry_id DESC LIMIT $5::integer
