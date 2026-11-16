      SELECT p.id,p.user_id,u.user_name,CASE WHEN p.apub THEN COALESCE(profile.display_name,u.display_name) ELSE u.display_name END AS display_name,p.content,p.created_at,p.updated_at,p.apub,p.visibility,p.channel_id,c.channel_name,false AS remote,NULL::text AS url,COALESCE(m.parent_id,p.parent_id) AS parent_id,NULL::jsonb AS emojis,'[]'::jsonb AS actor_emojis,(p.user_id=$1 OR p.reply_user_id=$1) AS own,true AS original_visible,NULL::jsonb AS avatar,u.avatar_url,p.media
      FROM /* local_posts */ p JOIN users u ON u.id=p.user_id LEFT JOIN apub_profiles profile ON profile.user_id=u.id LEFT JOIN channels c ON c.id=p.channel_id LEFT JOIN messages m ON m.id=p.id
      WHERE /* local_audience */
      UNION ALL
      SELECT p.id,p.actor_id,a.user_name || '@' || split_part(a.id,'/',3),a.display_name,p.content,p.published,p.updated_at,true,p.visibility,NULL,NULL,true,p.id,p.parent_id,p.emojis,a.emojis,false,
        (/* remote_original_audience */),a.avatar,NULL::text,p.media
      FROM /* remote_posts */ p JOIN apub_actors a ON a.id=p.actor_id
      WHERE p.deleted_at IS NULL AND NOT a.blocked AND (
        (/* remote_original_audience */)
        OR (p.visibility IN ('public','unlisted') AND EXISTS(
          SELECT 1 FROM apub_reposts r WHERE r.post_id=p.id AND (
            (r.user_id IS NOT NULL AND r.visibility IN ('local','public') AND (NOT $4::boolean OR r.user_id=$1))
            OR (r.user_id IS NULL AND EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1 OFFSET 0)
              AND r.actor_id IN (SELECT actor_id FROM apub_follows WHERE user_id=$1 AND direction='outgoing' AND accepted))
          ) OFFSET 0
        ))
      )
