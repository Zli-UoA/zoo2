ALTER TABLE messages ADD COLUMN card text;
ALTER TABLE users ADD COLUMN is_bot boolean NOT NULL DEFAULT false;
UPDATE users SET is_bot = true WHERE id IN (SELECT user_id FROM channel_webhooks);

-- Convert only messages written by known integration accounts, never human content.
UPDATE messages SET card = jsonb_build_object(
  'title', old.value->>'title',
  'description', COALESCE(old.value->>'description', ''),
  'url', COALESCE(old.value->>'url', ''),
  'source', 'GitHub',
  'context', COALESCE(old.value->>'repository', ''),
  'author', COALESCE(old.value->>'actor', ''),
  'badge', COALESCE(old.value->>'event', '')
)::text,
content = old.value->>'title'
FROM (
  SELECT id, CASE WHEN pg_input_is_valid(substring(content FROM 15), 'jsonb') THEN substring(content FROM 15)::jsonb ELSE NULL END AS value
  FROM messages
  WHERE user_id IN (SELECT user_id FROM channel_webhooks)
    AND left(content, 14) = E'zoo:github:v1\n'
    AND pg_input_is_valid(substring(content FROM 15), 'jsonb')
) old
WHERE messages.id = old.id AND jsonb_typeof(old.value->'title') = 'string';
