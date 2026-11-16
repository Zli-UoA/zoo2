blocked_actors AS MATERIALIZED (
 SELECT id FROM apub_actors WHERE blocked
), page_own_reactions AS MATERIALIZED (
 SELECT DISTINCT id,reaction,emoji FROM (
  SELECT parent.id,r.reaction,r.emoji FROM (SELECT DISTINCT id,channel_id FROM page) parent
  CROSS JOIN LATERAL (
   SELECT reaction,emoji,actor_id FROM apub_reactions WHERE post_id=parent.id AND user_id=$1 OFFSET 0
  ) r WHERE parent.channel_id IS NULL AND NOT EXISTS(SELECT 1 FROM blocked_actors WHERE id=r.actor_id OFFSET 0)
  UNION ALL
  SELECT parent.id,r.emoji,NULL::jsonb FROM (SELECT DISTINCT id,channel_id FROM page) parent
  CROSS JOIN LATERAL (
   SELECT emoji FROM message_reactions WHERE message_id=parent.id AND user_id=$1 OFFSET 0
  ) r WHERE parent.channel_id IS NOT NULL
 ) own
), blocked_reaction_groups AS MATERIALIZED (
 SELECT parent.id,CASE WHEN r.user_id IS NULL THEN 0 ELSE 1 END AS kind,
  home_reaction_key(r.reaction,r.emoji) AS key,count(*) AS n
 FROM (SELECT DISTINCT id FROM page) parent CROSS JOIN blocked_actors actor
 CROSS JOIN LATERAL (
  SELECT user_id,reaction,emoji FROM apub_reactions WHERE post_id=parent.id AND actor_id=actor.id OFFSET 0
 ) r GROUP BY parent.id,kind,key
), reaction_groups AS MATERIALIZED (
 SELECT id,reaction,emoji,sum(n)::bigint AS n,bool_or(selected) AS selected,min(first_at) AS first_at FROM (
  SELECT parent.id,c.reaction,c.emoji,c.n-COALESCE(blocked.n,0) AS n,own.id IS NOT NULL AS selected,
   CASE WHEN c.source_kind=2 THEN (
    SELECT created_at AT TIME ZONE 'UTC' FROM message_reactions r WHERE message_id=parent.id
     AND home_reaction_key(r.emoji,NULL::jsonb)=c.group_key ORDER BY created_at LIMIT 1
   ) ELSE (
    SELECT created_at FROM apub_reactions r WHERE post_id=parent.id AND (user_id IS NULL)=(c.source_kind=0)
     AND home_reaction_key(reaction,emoji)=c.group_key AND NOT EXISTS(SELECT 1 FROM blocked_actors WHERE id=r.actor_id OFFSET 0)
     ORDER BY created_at LIMIT 1
   ) END AS first_at
  FROM (SELECT DISTINCT id,channel_id FROM page) parent
  CROSS JOIN LATERAL (
   SELECT * FROM home_reaction_totals WHERE post_id=parent.id AND n>0
    AND (source_kind=0 OR source_kind=1 AND parent.channel_id IS NULL OR source_kind=2 AND parent.channel_id IS NOT NULL) OFFSET 0
  ) c
  LEFT JOIN blocked_reaction_groups blocked ON blocked.id=parent.id AND blocked.kind=c.source_kind AND blocked.key=c.group_key
  LEFT JOIN page_own_reactions own ON own.id=parent.id AND own.reaction=c.reaction AND own.emoji IS NOT DISTINCT FROM c.emoji
  WHERE c.n>COALESCE(blocked.n,0)
 ) visible_groups GROUP BY id,reaction,emoji
)
