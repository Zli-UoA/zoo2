SELECT candidate.*, a.user_name, a.display_name, a.emojis AS actor_emojis, a.avatar
FROM (
  SELECT p.id,p.actor_id,p.content,p.published,p.emojis,p.media
  FROM apub_remote_posts p
  WHERE p.id=target.id AND p.origin_verified AND p.deleted_at IS NULL AND p.visibility IN ('public','unlisted')
    AND NOT EXISTS(SELECT 1 FROM apub_quote_originals q WHERE q.id=p.id AND q.actor_id=p.actor_id AND (q.updated_at>p.updated_at OR (q.updated_at=p.updated_at AND NOT q.superseded_by_public)) OFFSET 0)
  UNION ALL
  SELECT q.id,q.actor_id,q.content,q.published,q.emojis,q.media
  FROM apub_quote_originals q
  WHERE q.id=target.id AND NOT q.superseded
    AND EXISTS(SELECT 1 FROM apub_quote_grants g WHERE g.post_id=q.id AND g.user_id=target.user_id AND g.note_revision=q.revision OFFSET 0)
    -- Bound the lookup by the whole follow primary key, including on generic plans
    -- where a popular actor otherwise looks cheap through the actor-only index.
    AND EXISTS(
      SELECT 1 FROM apub_follows f
      WHERE ROW(f.user_id,f.actor_id,f.direction) BETWEEN
        ROW(target.user_id,q.actor_id,'outgoing'::text) AND ROW(target.user_id,q.actor_id,'outgoing'::text)
        AND f.accepted
      ORDER BY f.user_id,f.actor_id,f.direction LIMIT 1
    )
    AND NOT EXISTS(SELECT 1 FROM apub_remote_posts p WHERE p.id=q.id AND (
      (p.actor_id<>q.actor_id AND p.origin_verified) OR (p.actor_id=q.actor_id AND (
        p.deleted_at IS NOT NULL OR p.updated_at>q.updated_at OR
        (p.updated_at=q.updated_at AND p.visibility='followers' AND q.observed_audience IS NULL AND (
          (p.content,p.published,p.parent_id,p.emojis) IS DISTINCT FROM (q.content,q.published,q.parent_id,q.emojis) OR
          ARRAY(SELECT item - 'blurhash' FROM jsonb_array_elements(p.media) WITH ORDINALITY AS m(item,ord) ORDER BY ord)
            <> ARRAY(SELECT item - 'blurhash' FROM jsonb_array_elements(q.media) WITH ORDINALITY AS m(item,ord) ORDER BY ord)
        ))
      ))
    ) OFFSET 0)
) candidate
JOIN apub_actors a ON a.id=candidate.actor_id AND NOT a.blocked
WHERE NOT EXISTS(SELECT 1 FROM apub_remote_tombstones t WHERE t.id=candidate.id AND t.actor_id=candidate.actor_id OFFSET 0)
