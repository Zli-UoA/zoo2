-- Page stable notification keys before constructing previews and emoji JSON.
WITH notice_page AS MATERIALIZED (
  SELECT n.id,n.user_id,n.actor_id,n.activity_id,n.notification_type AS kind,n.post_id,n.is_read,
    n.created_at AT TIME ZONE 'UTC' AS created_at,n.reaction,n.emoji
  FROM notification_events n
  WHERE n.source='activitypub' AND n.user_id=$1 AND ($2='' OR (n.created_at,n.id)<(
    SELECT created_at,id FROM notification_events WHERE source='activitypub' AND user_id=$1 AND id=$2
  ))
  AND (jsonb_array_length($3::jsonb)=0 OR n.id IN (SELECT jsonb_array_elements_text($3::jsonb)))
  ORDER BY n.created_at DESC,n.id DESC LIMIT 101
)
SELECT json_build_object('id',n.id,'actor_id',n.actor_id,'display_name',COALESCE(a.display_name,profile.display_name,u.display_name,n.actor_id),'actor_emojis',COALESCE(a.emojis,'[]'::jsonb),'avatar',a.avatar,'kind',n.kind,'activity_id',n.activity_id,'post_id',n.post_id,'reaction',n.reaction,'emoji',n.emoji,'content',CASE WHEN p.id IS NOT NULL AND p.deleted_at IS NULL THEN p.content WHEN rp.id IS NOT NULL AND rp.deleted_at IS NULL THEN rp.content ELSE NULL END,'content_emojis',CASE WHEN p.id IS NOT NULL AND p.deleted_at IS NULL THEN COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'width',e.value->'width','height',e.value->'height','local',true)) FROM jsonb_each(message_content_emojis(p.content)) e),'[]'::jsonb) WHEN rp.id IS NOT NULL AND rp.deleted_at IS NULL THEN rp.emojis ELSE '[]'::jsonb END,'is_read',n.is_read,'created_at',n.created_at,'pending',EXISTS(SELECT 1 FROM apub_follows f WHERE f.user_id=n.user_id AND f.actor_id=n.actor_id AND f.activity_id=n.activity_id AND f.direction='incoming' AND NOT f.accepted))::text
FROM notice_page n LEFT JOIN apub_actors a ON a.id=n.actor_id LEFT JOIN users u ON u.id=CASE WHEN starts_with(n.actor_id,'local:') THEN substr(n.actor_id,7) END LEFT JOIN apub_profiles profile ON profile.user_id=u.id LEFT JOIN home_posts p ON p.id=n.post_id AND (p.user_id=n.user_id OR p.reply_user_id=n.user_id) AND (p.channel_id IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=p.channel_id AND NOT c.is_private AND c.deleted_at IS NULL)) LEFT JOIN apub_remote_posts rp ON rp.id=n.post_id AND EXISTS(SELECT 1 FROM apub_remote_recipients recipient WHERE recipient.post_id=rp.id AND recipient.user_id=n.user_id)
ORDER BY n.created_at DESC,n.id DESC
