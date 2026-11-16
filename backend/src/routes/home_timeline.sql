-- Resolve keys through primary/parent indexes before constructing payloads.
-- Recursive reads are demand-limited, including very wide conversations.
WITH RECURSIVE visible AS NOT MATERIALIZED (
/* visible_posts */
), requested_reposts AS MATERIALIZED (
  SELECT substr(entry_id,8) AS activity_id FROM jsonb_array_elements_text($9::jsonb) ids(entry_id)
    WHERE $6='' AND starts_with(entry_id,'repost:')
  UNION SELECT substr($2,8) WHERE $6='' AND starts_with($2,'repost:')
), scoped_reposts AS MATERIALIZED (
  SELECT r.* FROM requested_reposts keys CROSS JOIN LATERAL (
    SELECT * FROM apub_reposts WHERE activity_id=keys.activity_id OFFSET 0
  ) r
), target_ids(id) AS MATERIALIZED (
  SELECT $6::text WHERE $6<>''
  UNION SELECT jsonb_array_elements_text($9::jsonb) WHERE $6=''
  UNION SELECT post_id FROM scoped_reposts
), ancestor_scope(id,parent_id,visited,depth) AS (
  SELECT b.id,b.parent_id,ARRAY[b.id],0 FROM target_ids t CROSS JOIN LATERAL (
    SELECT id,parent_id FROM visible WHERE id=t.id OFFSET 0
  ) b
  UNION ALL
  SELECT b.id,b.parent_id,a.visited||b.id,a.depth+1 FROM ancestor_scope a CROSS JOIN LATERAL (
    SELECT id,parent_id FROM visible WHERE id=a.parent_id AND original_visible OFFSET 0
  ) b WHERE NOT b.id=ANY(a.visited) AND a.depth<CASE WHEN $8::boolean THEN 100 ELSE 1 END
), conversation_ids(id) AS (
  SELECT id FROM ancestor_scope
  UNION
  SELECT b.id FROM conversation_ids t CROSS JOIN LATERAL (
    SELECT p.id FROM home_posts p LEFT JOIN messages m ON m.id=p.id WHERE p.parent_id=t.id AND COALESCE(m.parent_id,p.parent_id)=t.id
    UNION ALL SELECT p.id FROM messages m JOIN home_posts p ON p.id=m.id WHERE m.parent_id=t.id
    UNION ALL SELECT id FROM apub_remote_posts WHERE parent_id=t.id
  ) child CROSS JOIN LATERAL (
    SELECT id FROM visible WHERE id=child.id AND original_visible OFFSET 0
  ) b WHERE $8::boolean AND $6<>''
), thread_ids AS MATERIALIZED (
  -- PostgreSQL stops producing recursive rows when this consumer has enough.
  SELECT id FROM conversation_ids LIMIT CASE WHEN $8::boolean THEN $5::integer+101 ELSE $5::integer*2 END
), shared_reactions AS NOT MATERIALIZED (
      SELECT r.post_id,r.user_id,r.reaction,r.emoji,r.created_at FROM apub_reactions r
      WHERE (r.user_id IS NULL OR NOT EXISTS(SELECT 1 FROM home_posts p WHERE p.id=r.post_id AND p.channel_id IS NOT NULL)) AND NOT EXISTS(SELECT 1 FROM apub_actors a WHERE a.id=r.actor_id AND a.blocked)
      UNION ALL
      SELECT p.id,r.user_id,r.emoji,NULL::jsonb,r.created_at AT TIME ZONE 'UTC' FROM message_reactions r JOIN home_posts p ON p.id=r.message_id AND p.channel_id IS NOT NULL
    ), base AS MATERIALIZED (
      SELECT b.* FROM thread_ids t CROSS JOIN LATERAL (SELECT * FROM visible WHERE id=t.id OFFSET 0) b
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
/* timeline_entries */
    ), picked AS (
      SELECT f.extra,f.id,f.entry_id,f.sort_at,
        CASE WHEN $8::boolean AND (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN COALESCE(-a.depth,1) ELSE 0 END AS tree_order,
        CASE WHEN $8::boolean THEN d.path END AS tree_path,
        CASE WHEN $8::boolean AND (SELECT phase FROM thread_rollout WHERE id)<>'collecting' THEN LEAST(COALESCE(d.depth,0),1)
          WHEN $8::boolean THEN (SELECT max(depth) FROM ancestors)+COALESCE(-a.depth,d.depth,0) ELSE 0 END AS tree_depth
      FROM entries f
      LEFT JOIN ancestors a ON $8::boolean AND a.id=f.id
      LEFT JOIN descendants d ON $8::boolean AND d.id=f.id
      WHERE (jsonb_array_length($9::jsonb)=0 OR f.entry_id IN (SELECT jsonb_array_elements_text($9::jsonb)))
        AND ($6='' OR f.id=$6 OR $8::boolean AND (a.id IS NOT NULL OR d.id IS NOT NULL))
        AND ($7='' OR f.user_id=$7)
        AND ($2='' OR (f.sort_at,f.entry_id)<(SELECT sort_at,entry_id FROM entries WHERE entry_id=$2))
      ORDER BY CASE WHEN $8::boolean AND f.id=$6 THEN 0 WHEN $8::boolean AND a.id IS NOT NULL THEN 1 ELSE 2 END,
        tree_order,tree_path,f.sort_at DESC,f.entry_id DESC LIMIT $5::integer
    ), page AS MATERIALIZED (
      SELECT row_to_json(b) AS post,f.extra,b.channel_id,b.parent_id,
        CASE WHEN b.remote THEN NULL::json ELSE COALESCE((SELECT json_agg(json_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(b.content)) e),'[]'::json) END AS local_emojis,
        f.id,f.entry_id,f.sort_at,f.tree_order,f.tree_path,f.tree_depth
      FROM picked f JOIN base b ON b.id=f.id
    ), reaction_groups AS MATERIALIZED (
      SELECT parent.id,r.reaction,r.emoji,count(*) AS n,bool_or(r.user_id=$1) AS selected,
        min(r.created_at) AS first_at
      FROM (SELECT DISTINCT id FROM page) parent CROSS JOIN LATERAL (
        SELECT * FROM shared_reactions WHERE post_id=parent.id OFFSET 0
      ) r GROUP BY parent.id,r.reaction,r.emoji
    ), page_reactions AS MATERIALIZED (
      SELECT id,sum(n)::bigint AS likes,bool_or(selected) AS liked,
        (array_agg(reaction ORDER BY first_at) FILTER (WHERE selected))[1] AS my_reaction,
        jsonb_agg(jsonb_build_object('reaction',reaction,'emoji',emoji,'count',n,'selected',COALESCE(selected,false)) ORDER BY first_at) AS reactions
      FROM reaction_groups GROUP BY id
    ), page_reposts AS MATERIALIZED (
      SELECT parent.id,count(*) AS n,bool_or(r.user_id=$1) AS mine
      FROM (SELECT DISTINCT id FROM page) parent CROSS JOIN LATERAL (
        SELECT user_id FROM apub_reposts r WHERE r.post_id=parent.id AND (
          r.visibility IN ('public','local') OR r.user_id=$1 OR
          EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$1 OFFSET 0)
        ) OFFSET 0
      ) r GROUP BY parent.id
    ), page_parents AS MATERIALIZED (
      SELECT b.* FROM base b WHERE b.original_visible AND b.id IN (SELECT parent_id FROM page)
    ), page_parent_previews AS MATERIALIZED (
      SELECT parent.id,((row_to_json(parent)::jsonb || CASE WHEN parent.remote THEN '{}'::jsonb ELSE jsonb_build_object('emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(parent.content)) e),'[]'::jsonb)) END) - 'own' - 'original_visible' || jsonb_build_object('attachments',(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',a.id,'file_name',a.file_name,'file_size',a.file_size,'content_type',a.content_type,'width',a.width,'height',a.height,'blurhash',a.blurhash,'is_sensitive',a.is_sensitive) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM attachments a JOIN messages attachment_message ON attachment_message.id=a.message_id WHERE a.message_id=parent.id AND a.channel_id=parent.channel_id AND attachment_message.channel_id=parent.channel_id AND attachment_message.deleted_at IS NULL)))::json AS post FROM page_parents parent
    ), page_replies AS MATERIALIZED (
/* reply_counts */
    ) SELECT json_build_object('post',f.post,'display',json_build_object(
      'entry_id',f.entry_id,
      'attachments',(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',a.id,'file_name',a.file_name,'file_size',a.file_size,'content_type',a.content_type,'width',a.width,'height',a.height,'blurhash',a.blurhash,'is_sensitive',a.is_sensitive) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM attachments a JOIN messages attachment_message ON attachment_message.id=a.message_id WHERE a.message_id=f.id AND a.channel_id=f.channel_id AND attachment_message.channel_id=f.channel_id AND attachment_message.deleted_at IS NULL),
      'parent',parent.post,
      'tree_depth',f.tree_depth,
      'sort_key',(extract(epoch FROM f.sort_at)*1000000)::bigint::text,
      'likes',COALESCE(reactions.likes,0),
      'liked',COALESCE(reactions.liked,false),
      'my_reaction',reactions.my_reaction,
      'reactions',COALESCE(reactions.reactions,'[]'::jsonb),
      'replies',COALESCE(reply_stats.n,0),
      'reposts',COALESCE(reposts.n,0),
      'reposted',COALESCE(reposts.mine,false)
    ),'extra',f.extra,'local_emojis',f.local_emojis)::text FROM page f LEFT JOIN page_reactions reactions ON reactions.id=f.id LEFT JOIN page_reposts reposts ON reposts.id=f.id LEFT JOIN page_replies reply_stats ON reply_stats.parent_id=f.id LEFT JOIN page_parent_previews parent ON parent.id=f.parent_id ORDER BY f.tree_order,f.tree_path,f.sort_at DESC,f.entry_id DESC
