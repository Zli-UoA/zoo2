-- Reconcile legacy Home replies before edits can change their channel placement.
UPDATE messages m SET parent_id=p.parent_id
FROM home_posts p JOIN messages parent ON parent.id=p.parent_id AND parent.channel_id=p.channel_id
WHERE m.id=p.id AND m.channel_id=p.channel_id AND m.parent_id IS NULL;

-- Older replies outside the LTL pattern may not yet have a Home representation.
-- UNION deduplicates IDs even if corrupt legacy parent links contain a cycle.
WITH RECURSIVE eligible AS (
  SELECT m.* FROM messages m JOIN channels c ON c.id=m.channel_id
  WHERE m.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
), replies(id,visibility) AS (
  SELECT m.id,p.visibility FROM eligible m JOIN home_posts p
    ON p.id=m.parent_id AND p.channel_id=m.channel_id AND p.deleted_at IS NULL
  WHERE NOT EXISTS(SELECT 1 FROM home_posts old WHERE old.id=m.id)
  UNION
  SELECT m.id,r.visibility FROM eligible m JOIN replies r ON r.id=m.parent_id
  WHERE NOT EXISTS(SELECT 1 FROM home_posts old WHERE old.id=m.id)
)
INSERT INTO home_posts(id,user_id,content,channel_id,created_at,updated_at,parent_id,reply_user_id,visibility)
SELECT m.id,m.user_id,m.content,m.channel_id,m.created_at AT TIME ZONE 'UTC',m.updated_at AT TIME ZONE 'UTC',m.parent_id,parent.user_id,r.visibility
FROM replies r JOIN messages m ON m.id=r.id JOIN messages parent ON parent.id=m.parent_id AND parent.channel_id=m.channel_id
ON CONFLICT(id) DO NOTHING;
