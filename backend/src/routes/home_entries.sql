      SELECT '{}'::jsonb AS extra,b.id,b.id AS entry_id,b.created_at AS sort_at,b.user_id,NULL::text AS activity_id
      FROM /* entry_posts */ b WHERE (b.original_visible OR $6<>'') AND ($6<>'' OR $3='social' OR $3='local' AND NOT b.remote AND b.visibility IN ('public','local') OR $3='home' AND (b.remote OR b.own))
      UNION ALL
      SELECT jsonb_build_object('reposted_by',COALESCE(u.display_name::text,a.display_name),'reposter_actor',r.actor_id,'repost_created_at',r.created_at),b.id,'repost:'||r.activity_id,r.created_at,b.user_id,r.activity_id
      FROM apub_reposts r CROSS JOIN LATERAL (SELECT * FROM /* entry_posts */ WHERE id=r.post_id OFFSET 0) b LEFT JOIN users u ON u.id=r.user_id LEFT JOIN apub_actors a ON a.id=r.actor_id
      WHERE $6='' AND $7='' AND b.visibility IN ('public','unlisted','local') AND (r.user_id IS NOT NULL OR NOT a.blocked) AND (
        (r.user_id=$1 AND $3 IN ('home','social'))
        OR (r.user_id IS NOT NULL AND r.visibility IN ('public','local') AND $3 IN ('local','social') AND (NOT $4::boolean OR r.user_id=$1))
        OR (r.user_id IS NULL AND $3 IN ('home','social') AND EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1) AND r.actor_id IN (SELECT actor_id FROM apub_follows WHERE user_id=$1 AND direction='outgoing' AND accepted))
      )
