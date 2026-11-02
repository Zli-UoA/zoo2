
-- Keep legacy local Home reactions available in the channel during rolling upgrades.
CREATE FUNCTION sync_linked_post_reaction() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP <> 'INSERT' AND OLD.user_id IS NOT NULL THEN
    IF TG_OP = 'DELETE' OR (OLD.post_id,OLD.user_id,OLD.reaction) IS DISTINCT FROM (NEW.post_id,NEW.user_id,NEW.reaction) THEN
      DELETE FROM message_reactions r USING home_posts p WHERE p.id=OLD.post_id AND p.channel_id IS NOT NULL
        AND r.message_id=p.id AND r.user_id=OLD.user_id AND r.emoji=OLD.reaction;
    END IF;
  END IF;
  IF TG_OP <> 'DELETE' AND NEW.user_id IS NOT NULL THEN
    INSERT INTO message_reactions(id,message_id,user_id,emoji,created_at)
      SELECT gen_random_uuid()::text,p.id,NEW.user_id,NEW.reaction,NEW.created_at AT TIME ZONE 'UTC'
      FROM home_posts p JOIN messages m ON m.id=p.id WHERE p.id=NEW.post_id AND p.channel_id IS NOT NULL
        AND p.deleted_at IS NULL AND m.deleted_at IS NULL
      ON CONFLICT(message_id,user_id,emoji) DO NOTHING;
  END IF;
  RETURN NULL;
END $$;
CREATE TRIGGER linked_post_reaction_sync AFTER INSERT OR UPDATE OR DELETE ON apub_reactions
  FOR EACH ROW EXECUTE FUNCTION sync_linked_post_reaction();

CREATE FUNCTION notify_linked_message_reaction() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS(SELECT 1 FROM home_posts WHERE id=CASE WHEN TG_OP='DELETE' THEN OLD.message_id ELSE NEW.message_id END AND channel_id IS NOT NULL) THEN
    PERFORM pg_notify('home_timeline','');
  END IF;
  RETURN NULL;
END $$;
CREATE TRIGGER linked_message_reaction_live AFTER INSERT OR DELETE ON message_reactions
  FOR EACH ROW EXECUTE FUNCTION notify_linked_message_reaction();

-- Existing local reactions are copied without replacing native reaction identities.
INSERT INTO message_reactions(id,message_id,user_id,emoji,created_at)
  SELECT gen_random_uuid()::text,p.id,r.user_id,r.reaction,r.created_at AT TIME ZONE 'UTC'
  FROM apub_reactions r JOIN home_posts p ON p.id=r.post_id JOIN messages m ON m.id=p.id
  WHERE p.channel_id IS NOT NULL AND p.deleted_at IS NULL AND m.deleted_at IS NULL AND r.user_id IS NOT NULL
  ON CONFLICT(message_id,user_id,emoji) DO NOTHING;
