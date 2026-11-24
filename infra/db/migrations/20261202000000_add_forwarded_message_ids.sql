-- メッセージ転送（引用）機能の参照先 ID 配列。NULL 許容・FK なしで後方互換と Slack 順不同取り込みを保つ。
ALTER TABLE messages ADD COLUMN forwarded_message_ids jsonb;

CREATE OR REPLACE FUNCTION messages_clear_deleted_content() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.content := '';
  NEW.card := NULL;
  NEW.components := NULL;
  NEW.metadata := NULL;
  NEW.display_data := '{}'::jsonb;
  NEW.app_blocks := NULL;
  NEW.app_attachments := NULL;
  NEW.app_unfurls := NULL;
  NEW.forwarded_message_ids := NULL;
  RETURN NEW;
END $$;
DROP TRIGGER messages_clear_deleted_content ON messages;
CREATE TRIGGER messages_clear_deleted_content BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL AND (
    NEW.content <> '' OR NEW.card IS NOT NULL OR NEW.components IS NOT NULL OR NEW.metadata IS NOT NULL
    OR NEW.display_data <> '{}'::jsonb
    OR NEW.app_blocks IS NOT NULL OR NEW.app_attachments IS NOT NULL OR NEW.app_unfurls IS NOT NULL
    OR NEW.forwarded_message_ids IS NOT NULL))
  EXECUTE FUNCTION messages_clear_deleted_content();

CREATE OR REPLACE FUNCTION e2ee_reject_plaintext_messages() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  enc boolean;
BEGIN
  -- encrypted の条件を WHERE に入れると、切り替える前の版で条件に合わない行はロックされず、切り替えを待たない。
  SELECT encrypted INTO enc FROM channels WHERE id = NEW.channel_id FOR KEY SHARE;
  IF NOT COALESCE(enc, false) THEN
    RETURN NEW;
  END IF;
  -- 別のチャンネルから移した行は、切り替える前からこのチャンネルにあった行ではないので、例外にしない。
  IF TG_OP = 'UPDATE' AND OLD.channel_id = NEW.channel_id AND OLD.epoch IS NULL AND OLD.ciphertext IS NULL THEN
    IF NEW.ciphertext IS NOT NULL OR NEW.epoch IS NOT NULL
       OR (NEW.content IS DISTINCT FROM OLD.content AND NEW.content <> '')
       OR (NULLIF(NEW.app_attachments, '') IS DISTINCT FROM NULLIF(OLD.app_attachments, '') AND NULLIF(NEW.app_attachments, '') IS NOT NULL)
       OR (NULLIF(NEW.app_blocks, '') IS DISTINCT FROM NULLIF(OLD.app_blocks, '') AND NULLIF(NEW.app_blocks, '') IS NOT NULL)
       OR (NULLIF(NEW.app_unfurls, '') IS DISTINCT FROM NULLIF(OLD.app_unfurls, '') AND NULLIF(NEW.app_unfurls, '') IS NOT NULL)
       OR (NULLIF(NEW.metadata, '') IS DISTINCT FROM NULLIF(OLD.metadata, '') AND NULLIF(NEW.metadata, '') IS NOT NULL)
       OR (NULLIF(NEW.card, '') IS DISTINCT FROM NULLIF(OLD.card, '') AND NULLIF(NEW.card, '') IS NOT NULL)
       OR (NULLIF(NEW.components, '') IS DISTINCT FROM NULLIF(OLD.components, '') AND NULLIF(NEW.components, '') IS NOT NULL)
       OR (NEW.forwarded_message_ids IS DISTINCT FROM OLD.forwarded_message_ids AND NEW.forwarded_message_ids IS NOT NULL)
       OR (NEW.home_post_id IS DISTINCT FROM OLD.home_post_id AND NEW.home_post_id IS NOT NULL)
       OR (NEW.apub AND NOT OLD.apub)
       OR (OLD.deleted_at IS NOT NULL AND NEW.deleted_at IS NULL) THEN
      RAISE EXCEPTION 'messages written before the channel was encrypted cannot be changed to carry new content' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
  END IF;
  -- 削除済みの行は本文が空になる（messages_clear_deleted_content）。それでも、別のチャンネルから暗号文のない
  -- 行を移してくることは、削除済みかどうかにかかわらず認めない。
  IF NEW.content <> '' OR (NEW.deleted_at IS NULL AND NEW.ciphertext IS NULL)
     OR (TG_OP = 'UPDATE' AND OLD.channel_id IS DISTINCT FROM NEW.channel_id AND NEW.ciphertext IS NULL) THEN
    RAISE EXCEPTION 'messages in an encrypted channel require ciphertext and empty content' USING ERRCODE = '23514';
  END IF;
  IF NULLIF(NEW.app_attachments, '') IS NOT NULL OR NULLIF(NEW.app_blocks, '') IS NOT NULL
     OR NULLIF(NEW.app_unfurls, '') IS NOT NULL OR NULLIF(NEW.metadata, '') IS NOT NULL
     OR NULLIF(NEW.card, '') IS NOT NULL OR NULLIF(NEW.components, '') IS NOT NULL
     OR NEW.forwarded_message_ids IS NOT NULL
     -- Home との同期（sync_home_post）は、削除済みの行を INSERT ... ON CONFLICT で書き戻す。削除済みの行は本文が空なので、
     -- Home との紐付けだけは認める。
     OR (NEW.deleted_at IS NULL AND (NEW.home_post_id IS NOT NULL OR NEW.apub)) THEN
    RAISE EXCEPTION 'messages in an encrypted channel cannot carry plaintext fields' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE VIEW notification_display AS
SELECT n.id, jsonb_build_object(
  'message', CASE WHEN m.deleted_at IS NULL AND m.id IS NOT NULL THEN
    jsonb_build_object('id',m.id,'user_id',m.user_id,'content',left(m.content,281),
      'has_forwards',(m.forwarded_message_ids IS NOT NULL AND jsonb_typeof(m.forwarded_message_ids)='array' AND jsonb_array_length(m.forwarded_message_ids)>0),
      'emojis',COALESCE((SELECT jsonb_object_agg(e.key,e.value)
        FROM jsonb_each(COALESCE(m.display_data->'emojis','{}'::jsonb)) e
        WHERE strpos(left(m.content,281),':'||e.key||':')>0 OR
          badge.reaction->>'emoji' IN (e.key,':'||e.key||':')), '{}'::jsonb)) ELSE NULL END,
  'reactions', CASE WHEN n.notification_type='reaction' AND m.deleted_at IS NULL AND badge.reaction IS NOT NULL
    THEN jsonb_build_array(badge.reaction) ELSE '[]'::jsonb END,
  'actor', jsonb_build_object('id',u.id,'user_name',u.user_name,'display_name',u.display_name,
    'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id),
  'channel', jsonb_build_object('id',c.id,'channel_name',c.channel_name,'description',c.description,'created_user_id',c.created_user_id,'is_private',c.is_private,'is_dm',c.is_dm,'created_at',c.created_at,'updated_at',c.updated_at,'archive_at',c.archive_at,'deleted_at',c.deleted_at,'imported_from_slack',c.imported_from_slack,'slack_import_kind',c.slack_import_kind,'slack_imported_at',c.slack_imported_at,'topic',c.topic,'posting_policy',c.posting_policy,'encrypted',c.encrypted,'is_nsfw',c.is_nsfw)
) AS display_data
FROM notification_events n JOIN users u ON u.id=n.actor_id JOIN channels c ON c.id=n.channel_id
LEFT JOIN messages m ON m.id=n.message_id AND m.channel_id=n.channel_id
LEFT JOIN LATERAL (
  SELECT entry.value AS reaction FROM jsonb_array_elements(COALESCE(m.display_data->'reactions','[]'::jsonb)) AS entry(value)
  WHERE n.notification_type='reaction' AND entry.value->>'user_id'=n.actor_id
  ORDER BY entry.value->>'created_at' DESC,entry.value->>'id' LIMIT 1
) badge ON true
WHERE n.source='channel';

CREATE OR REPLACE FUNCTION sync_message_notifications_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF current_setting('zoo.display_refresh',true)='metadata' THEN
    INSERT INTO display_refresh_queue(kind,target) SELECT 'notification',id FROM notification_events
      WHERE source='channel' AND message_id=NEW.id ORDER BY id ON CONFLICT DO NOTHING;
    RETURN NULL;
  END IF;
  FOR target IN SELECT n.id FROM notification_events n
    WHERE n.source='channel' AND n.message_id=NEW.id AND (
      OLD.content IS DISTINCT FROM NEW.content OR OLD.user_id IS DISTINCT FROM NEW.user_id OR
      OLD.deleted_at IS DISTINCT FROM NEW.deleted_at OR
      OLD.forwarded_message_ids IS DISTINCT FROM NEW.forwarded_message_ids OR
      (OLD.display_data-'reactions') IS DISTINCT FROM (NEW.display_data-'reactions') OR
      n.notification_type='reaction' AND n.actor_id IN (
        SELECT reaction->>'user_id' FROM (
          (SELECT value AS reaction FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]')))
          UNION
          (SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]')))
        ) changed
      )
    ) ORDER BY n.id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS message_notifications_display ON messages;
CREATE TRIGGER message_notifications_display AFTER UPDATE OF content,user_id,display_data,deleted_at,forwarded_message_ids ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_notifications_display();
