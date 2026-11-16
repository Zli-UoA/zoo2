-- Count directly through parent indexes; do not resolve a complete visible
-- post (and repeat its PK/actor/audience joins) separately for every reply.
SELECT parent.id AS parent_id,local_replies.n+remote_replies.n AS n
FROM (SELECT DISTINCT id FROM page) parent
CROSS JOIN LATERAL (
  SELECT count(*) AS n FROM (
    SELECT p.* FROM home_posts p LEFT JOIN messages m ON m.id=p.id
      WHERE p.parent_id=parent.id AND COALESCE(m.parent_id,p.parent_id)=parent.id
    UNION ALL
    SELECT p.* FROM messages m JOIN home_posts p ON p.id=m.id
      WHERE m.parent_id=parent.id AND p.parent_id IS DISTINCT FROM parent.id
  ) p LEFT JOIN channels c ON c.id=p.channel_id
  WHERE /* local_audience */
) local_replies
CROSS JOIN LATERAL (
  SELECT COALESCE(sum(c.n),0)::bigint AS n FROM apub_reply_counts c JOIN apub_actors a ON a.id=c.actor_id
  WHERE c.user_id=$1 AND c.parent_id=parent.id AND NOT a.blocked AND (
    c.actor_id IN (SELECT actor_id FROM apub_follows WHERE user_id=$1 AND direction='outgoing' AND accepted)
    OR EXISTS(SELECT 1 FROM home_posts owner WHERE owner.id=parent.id AND owner.user_id=$1 OFFSET 0)
  )
) remote_replies
