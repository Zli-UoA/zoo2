-- Resolve keys through primary/parent indexes before constructing payloads.
-- Recursive reads are demand-limited, including very wide conversations.
WITH RECURSIVE visible AS NOT MATERIALIZED (
/* visible_posts */
), target_ids(id) AS MATERIALIZED (
  SELECT $6::text WHERE $6<>''
  UNION SELECT jsonb_array_elements_text($9::jsonb) WHERE $6=''
  UNION SELECT post_id FROM apub_reposts WHERE $6='' AND 'repost:'||activity_id IN (SELECT jsonb_array_elements_text($9::jsonb))
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
    UNION SELECT p.id FROM messages m JOIN home_posts p ON p.id=m.id WHERE m.parent_id=t.id
    UNION SELECT id FROM apub_remote_posts WHERE parent_id=t.id
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
      SELECT f.extra,f.id,f.entry_id,f.sort_at,CASE WHEN $8::boolean AND (SELECT phase FROM thread_rollout WHERE id)='collecting' THEN COALESCE((SELECT -depth FROM ancestors WHERE id=f.id),1) ELSE 0 END AS tree_order,CASE WHEN $8::boolean THEN (SELECT path FROM descendants WHERE id=f.id) END AS tree_path
      FROM entries f WHERE (jsonb_array_length($9::jsonb)=0 OR f.entry_id IN (SELECT jsonb_array_elements_text($9::jsonb))) AND ($6='' OR f.id=$6 OR $8::boolean AND (f.id IN (SELECT id FROM ancestors) OR f.id IN (SELECT id FROM descendants))) AND ($7='' OR f.user_id=$7) AND ($2='' OR (f.sort_at,f.entry_id)<(SELECT sort_at,entry_id FROM entries WHERE entry_id=$2))
      ORDER BY CASE WHEN $8::boolean AND f.id=$6 THEN 0 WHEN $8::boolean AND f.id IN (SELECT id FROM ancestors) THEN 1 ELSE 2 END,tree_order,tree_path,f.sort_at DESC,f.entry_id DESC LIMIT $5::integer
    ), page AS MATERIALIZED (
      SELECT row_to_json(b)::jsonb || CASE WHEN b.remote THEN '{}'::jsonb ELSE jsonb_build_object('emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(b.content)) e),'[]'::jsonb)) END || f.extra AS post,f.id,f.entry_id,f.sort_at,f.tree_order,f.tree_path
      FROM picked f JOIN base b ON b.id=f.id
    ), page_parents AS MATERIALIZED (
      SELECT b.* FROM base b WHERE b.original_visible AND b.id IN (SELECT post->>'parent_id' FROM page)
    ), page_replies AS MATERIALIZED (
      SELECT parent.id AS parent_id,count(*) AS n FROM (SELECT DISTINCT id FROM page) parent CROSS JOIN LATERAL (
        SELECT p.id FROM home_posts p LEFT JOIN messages m ON m.id=p.id WHERE p.parent_id=parent.id AND COALESCE(m.parent_id,p.parent_id)=parent.id
        UNION SELECT p.id FROM messages m JOIN home_posts p ON p.id=m.id WHERE m.parent_id=parent.id
        UNION SELECT id FROM apub_remote_posts WHERE parent_id=parent.id
      ) child CROSS JOIN LATERAL (
        SELECT id FROM visible WHERE id=child.id AND original_visible OFFSET 0
      ) b GROUP BY parent.id
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
