SELECT parent.id,COALESCE(public.n,0)+own.private_n+COALESCE(received.n,0) AS n,own.mine
FROM (SELECT DISTINCT id FROM page) parent
LEFT JOIN LATERAL (SELECT n FROM home_repost_totals WHERE post_id=parent.id OFFSET 0) public ON true
LEFT JOIN LATERAL (SELECT n FROM home_private_repost_totals WHERE user_id=$1 AND post_id=parent.id OFFSET 0) received ON true
CROSS JOIN LATERAL (
 SELECT count(*) FILTER(WHERE visibility NOT IN ('public','local')) AS private_n,count(*)>0 AS mine
 FROM apub_reposts WHERE user_id=$1 AND post_id=parent.id OFFSET 0
) own
