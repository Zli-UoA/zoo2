-- The scope discovers IDs only; `base` below still applies every audience,
-- deletion, channel and block check. UNION deduplicates IDs and stops cycles.
-- Traverse both parent sources because messages may override a Home parent.
-- Include relatives outside the rendered depth/limit for previews and counts.
-- Detail reads recurse through the conversation; author/viewport seeds expand
-- one hop only, so unrelated conversations never reach the visible base.
WITH RECURSIVE target_ids(id) AS MATERIALIZED (
      SELECT $6::text WHERE $6<>''
      UNION
      SELECT id FROM home_posts WHERE $6='' AND jsonb_array_length($9::jsonb)=0 AND $7<>'' AND user_id=$7
      UNION
      SELECT id FROM apub_remote_posts WHERE $6='' AND jsonb_array_length($9::jsonb)=0 AND $7<>'' AND actor_id=$7
      UNION
      SELECT jsonb_array_elements_text($9::jsonb) WHERE $6=''
      UNION
      SELECT r.post_id FROM apub_reposts r WHERE $6='' AND 'repost:'||r.activity_id IN (SELECT jsonb_array_elements_text($9::jsonb))
    ), thread_ids(id) AS (
      SELECT id FROM target_ids
      UNION
      SELECT related.id FROM thread_ids t CROSS JOIN LATERAL (
        SELECT COALESCE(m.parent_id,p.parent_id) AS id FROM home_posts p LEFT JOIN messages m ON m.id=p.id WHERE p.id=t.id
        UNION ALL
        SELECT p.parent_id FROM apub_remote_posts p WHERE p.id=t.id
        UNION ALL
        SELECT p.id FROM home_posts p LEFT JOIN messages m ON m.id=p.id WHERE p.parent_id=t.id AND COALESCE(m.parent_id,p.parent_id)=t.id
        UNION ALL
        SELECT p.id FROM messages m JOIN home_posts p ON p.id=m.id WHERE m.parent_id=t.id
        UNION ALL
        SELECT p.id FROM apub_remote_posts p WHERE p.parent_id=t.id
      ) related WHERE related.id IS NOT NULL AND ($6<>'' OR t.id IN (SELECT id FROM target_ids))
    ), scoped_home_posts AS NOT MATERIALIZED (
      SELECT p.* FROM thread_ids scope CROSS JOIN LATERAL (
        SELECT * FROM home_posts WHERE id=scope.id OFFSET 0
      ) p
    ), scoped_remote_posts AS NOT MATERIALIZED (
      SELECT p.* FROM thread_ids scope CROSS JOIN LATERAL (
        SELECT * FROM apub_remote_posts WHERE id=scope.id OFFSET 0
      ) p
    ), shared_reactions AS NOT MATERIALIZED (
      SELECT r.post_id,r.user_id,r.reaction,r.emoji,r.created_at FROM apub_reactions r
      WHERE (r.user_id IS NULL OR NOT EXISTS(SELECT 1 FROM home_posts p WHERE p.id=r.post_id AND p.channel_id IS NOT NULL)) AND NOT EXISTS(SELECT 1 FROM apub_actors a WHERE a.id=r.actor_id AND a.blocked)
      UNION ALL
      SELECT p.id,r.user_id,r.emoji,NULL::jsonb,r.created_at AT TIME ZONE 'UTC' FROM message_reactions r JOIN home_posts p ON p.id=r.message_id AND p.channel_id IS NOT NULL
    ), base AS (
      SELECT p.id,p.user_id,u.user_name,CASE WHEN p.apub THEN COALESCE(profile.display_name,u.display_name) ELSE u.display_name END AS display_name,p.content,p.created_at,p.updated_at,p.apub,p.visibility,p.channel_id,c.channel_name,false AS remote,NULL::text AS url,COALESCE(m.parent_id,p.parent_id) AS parent_id,NULL::jsonb AS emojis,'[]'::jsonb AS actor_emojis,(p.user_id=$1 OR p.reply_user_id=$1) AS own,true AS original_visible,NULL::jsonb AS avatar,u.avatar_url,p.media
      FROM /* local_posts */ p JOIN users u ON u.id=p.user_id LEFT JOIN apub_profiles profile ON profile.user_id=u.id LEFT JOIN channels c ON c.id=p.channel_id LEFT JOIN messages m ON m.id=p.id
      WHERE p.deleted_at IS NULL AND (p.visibility IN ('local','public') OR p.user_id=$1 OR p.reply_user_id=$1 OR (p.id=$6 AND p.apub AND p.visibility='unlisted'))
        AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)
        AND (NOT $4::boolean OR p.user_id=$1 OR p.reply_user_id=$1 OR EXISTS(SELECT 1 FROM channel_user WHERE channel_id=p.channel_id AND user_id=$1))
      UNION ALL
      SELECT p.id,p.actor_id,a.user_name || '@' || split_part(a.id,'/',3),a.display_name,p.content,p.published,p.updated_at,true,p.visibility,NULL,NULL,true,p.id,p.parent_id,p.emojis,a.emojis,false,
        (EXISTS(SELECT 1 FROM apub_remote_recipients WHERE post_id=p.id AND user_id=$1) AND (EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$1 AND actor_id=p.actor_id AND direction='outgoing' AND accepted) OR EXISTS(SELECT 1 FROM home_posts parent WHERE parent.id=p.parent_id AND parent.user_id=$1))),a.avatar,NULL::text,p.media
      FROM /* remote_posts */ p JOIN apub_actors a ON a.id=p.actor_id
      WHERE p.deleted_at IS NULL AND NOT a.blocked AND (
        (EXISTS(SELECT 1 FROM apub_remote_recipients WHERE post_id=p.id AND user_id=$1) AND (EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$1 AND actor_id=p.actor_id AND direction='outgoing' AND accepted) OR EXISTS(SELECT 1 FROM home_posts parent WHERE parent.id=p.parent_id AND parent.user_id=$1)))
        OR (p.visibility IN ('public','unlisted') AND EXISTS(SELECT 1 FROM apub_reposts r WHERE r.post_id=p.id AND ((r.user_id IS NOT NULL AND r.visibility IN ('local','public') AND (NOT $4::boolean OR r.user_id=$1)) OR (r.user_id IS NULL AND EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1) AND EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$1 AND actor_id=r.actor_id AND direction='outgoing' AND accepted)))))
      )
    ), ancestors AS (
      SELECT b.id,b.parent_id,ARRAY[b.id] AS visited,0 AS depth FROM base b WHERE b.id=$6
      UNION ALL
      SELECT b.id,b.parent_id,a.visited||b.id,a.depth+1 FROM ancestors a JOIN base b ON b.id=a.parent_id
      WHERE NOT b.id=ANY(a.visited) AND a.depth<100 AND b.original_visible
    ), descendants AS (
      SELECT b.id,ARRAY[b.id] AS visited,ARRAY[b.created_at::text||b.id] AS path,0 AS depth FROM base b WHERE b.id=CASE WHEN (SELECT phase FROM thread_rollout WHERE id)<>'collecting' THEN (SELECT id FROM ancestors ORDER BY depth DESC LIMIT 1) ELSE $6 END
      UNION ALL
      SELECT b.id,d.visited||b.id,d.path||(b.created_at::text||b.id),d.depth+1 FROM descendants d JOIN base b ON b.parent_id=d.id
      WHERE NOT b.id=ANY(d.visited) AND d.depth<100 AND b.original_visible
    ), entries AS (
      SELECT '{}'::jsonb AS extra,b.id,b.id AS entry_id,b.created_at AS sort_at,b.user_id
      FROM base b WHERE (b.original_visible OR $6<>'') AND ($6<>'' OR $3='social' OR $3='local' AND NOT b.remote AND b.visibility IN ('public','local') OR $3='home' AND (b.remote OR b.own))
      UNION ALL
      SELECT jsonb_build_object('reposted_by',COALESCE(u.display_name::text,a.display_name),'reposter_actor',r.actor_id,'repost_created_at',r.created_at),b.id,'repost:'||r.activity_id,r.created_at,b.user_id
      FROM apub_reposts r JOIN base b ON b.id=r.post_id LEFT JOIN users u ON u.id=r.user_id LEFT JOIN apub_actors a ON a.id=r.actor_id
      WHERE $6='' AND $7='' AND b.visibility IN ('public','unlisted','local') AND (r.user_id IS NOT NULL OR NOT a.blocked) AND (
        (r.user_id=$1 AND $3 IN ('home','social'))
        OR (r.user_id IS NOT NULL AND r.visibility IN ('public','local') AND $3 IN ('local','social') AND (NOT $4::boolean OR r.user_id=$1))
        OR (r.user_id IS NULL AND $3 IN ('home','social') AND EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1) AND EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$1 AND actor_id=r.actor_id AND direction='outgoing' AND accepted))
      )
    ), picked AS (
      SELECT f.extra,f.id,f.entry_id,f.sort_at,CASE WHEN $8::boolean AND (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN COALESCE((SELECT -depth FROM ancestors WHERE id=f.id),1) ELSE 0 END AS tree_order,CASE WHEN $8::boolean THEN (SELECT path FROM descendants WHERE id=f.id) END AS tree_path
      FROM entries f WHERE (jsonb_array_length($9::jsonb)=0 OR f.entry_id IN (SELECT jsonb_array_elements_text($9::jsonb))) AND ($6='' OR f.id=$6 OR $8::boolean AND (f.id IN (SELECT id FROM ancestors) OR f.id IN (SELECT id FROM descendants))) AND ($7='' OR f.user_id=$7) AND ($2='' OR (f.sort_at,f.entry_id)<(SELECT sort_at,entry_id FROM entries WHERE entry_id=$2))
      ORDER BY CASE WHEN $8::boolean AND f.id=$6 THEN 0 WHEN $8::boolean AND f.id IN (SELECT id FROM ancestors) THEN 1 ELSE 2 END,tree_order,tree_path,f.sort_at DESC,f.entry_id DESC LIMIT $5::integer
    ), page AS MATERIALIZED (
      SELECT row_to_json(b)::jsonb || CASE WHEN b.remote THEN '{}'::jsonb ELSE jsonb_build_object('emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(b.content)) e),'[]'::jsonb)) END || f.extra AS post,f.id,f.entry_id,f.sort_at,f.tree_order,f.tree_path
      FROM picked f JOIN base b ON b.id=f.id
    ), page_parents AS MATERIALIZED (
      SELECT b.* FROM base b WHERE b.original_visible AND b.id IN (SELECT post->>'parent_id' FROM page)
    ), page_replies AS MATERIALIZED (
      SELECT b.parent_id,count(*) AS n FROM base b WHERE b.original_visible AND b.parent_id IN (SELECT id FROM page) GROUP BY b.parent_id
    ) SELECT (f.post - 'own' - 'original_visible' || jsonb_build_object(
      'entry_id',f.entry_id,
      'attachments',(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',a.id,'file_name',a.file_name,'file_size',a.file_size,'content_type',a.content_type,'width',a.width,'height',a.height,'blurhash',a.blurhash,'is_sensitive',a.is_sensitive) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM attachments a JOIN messages attachment_message ON attachment_message.id=a.message_id WHERE a.message_id=f.id AND a.channel_id=f.post->>'channel_id' AND attachment_message.channel_id=f.post->>'channel_id' AND attachment_message.deleted_at IS NULL),
      'parent',(SELECT (row_to_json(parent)::jsonb || CASE WHEN parent.remote THEN '{}'::jsonb ELSE jsonb_build_object('emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(parent.content)) e),'[]'::jsonb)) END) - 'own' - 'original_visible' || jsonb_build_object('attachments',(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',a.id,'file_name',a.file_name,'file_size',a.file_size,'content_type',a.content_type,'width',a.width,'height',a.height,'blurhash',a.blurhash,'is_sensitive',a.is_sensitive) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM attachments a JOIN messages attachment_message ON attachment_message.id=a.message_id WHERE a.message_id=parent.id AND a.channel_id=parent.channel_id AND attachment_message.channel_id=parent.channel_id AND attachment_message.deleted_at IS NULL)) FROM page_parents parent WHERE parent.id=f.post->>'parent_id'),
      'tree_depth',CASE WHEN $8::boolean AND (SELECT phase FROM thread_rollout WHERE id)<>'collecting' THEN LEAST(COALESCE((SELECT depth FROM descendants WHERE id=f.id),0),1) WHEN $8::boolean THEN (SELECT max(depth) FROM ancestors) + COALESCE((SELECT -depth FROM ancestors WHERE id=f.id),(SELECT depth FROM descendants WHERE id=f.id),0) ELSE 0 END,
      'sort_key',(extract(epoch FROM f.sort_at)*1000000)::bigint::text,
      'likes',(SELECT count(*) FROM shared_reactions WHERE post_id=f.id),
      'liked',EXISTS(SELECT 1 FROM shared_reactions WHERE post_id=f.id AND user_id=$1),
      'my_reaction',(SELECT reaction FROM shared_reactions WHERE post_id=f.id AND user_id=$1 LIMIT 1),
      'reactions',(SELECT COALESCE(jsonb_agg(jsonb_build_object('reaction',reaction,'emoji',emoji,'count',n,'selected',COALESCE(selected,false))),'[]'::jsonb) FROM (SELECT reaction,emoji,count(*) AS n,bool_or(user_id=$1) AS selected FROM shared_reactions WHERE post_id=f.id GROUP BY reaction,emoji ORDER BY min(created_at)) grouped),
      'replies',COALESCE((SELECT n FROM page_replies WHERE parent_id=f.id),0),
      'reposts',(SELECT count(*) FROM apub_reposts r WHERE r.post_id=f.id AND (r.visibility IN ('public','local') OR r.user_id=$1 OR EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1))),
      'reposted',EXISTS(SELECT 1 FROM apub_reposts WHERE post_id=f.id AND user_id=$1)
    ))::text FROM page f ORDER BY f.tree_order,f.tree_path,f.sort_at DESC,f.entry_id DESC
