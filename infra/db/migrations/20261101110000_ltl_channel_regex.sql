-- Saved values are used as regular expressions as-is; no conversion or rewrite.
CREATE FUNCTION valid_ltl_channel_regex(pattern text) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
  PERFORM '' ~ pattern;
  RETURN true;
EXCEPTION WHEN invalid_regular_expression THEN
  RETURN false;
END $$;
CREATE FUNCTION channel_matches_ltl(channel_name text, pattern text) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
  RETURN pattern<>'' AND channel_name ~ pattern;
EXCEPTION WHEN invalid_regular_expression THEN
  RETURN false;
END $$;
CREATE OR REPLACE FUNCTION sync_channel_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC');
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND channel_matches_ltl(c.channel_name,s.ltl_channel_pattern)
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC') ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;
