-- E2EE: 暗号化チャンネルで平文に依存する機能を止める（#358。仕様は docs/e2ee.md の「DB」と「機能ごとの扱い」）。

-- 暗号化チャンネルの messages に平文が入らないようにする。API 以外の経路（Webhook、App と委任トークン、
-- Slack 取り込み、Home の同期など）で書き込まれても止める最後の防御。論理削除した行の ciphertext は
-- messages_clear_deleted_ciphertext（名前の順でこれより先に動く）が消す。
CREATE FUNCTION e2ee_reject_plaintext_messages() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM channels WHERE id = NEW.channel_id AND encrypted) THEN
    RETURN NEW;
  END IF;
  IF NEW.deleted_at IS NULL AND (NEW.content <> '' OR NEW.ciphertext IS NULL) THEN
    RAISE EXCEPTION 'messages in an encrypted channel require ciphertext and empty content' USING ERRCODE = '23514';
  END IF;
  IF NULLIF(NEW.app_attachments, '') IS NOT NULL OR NULLIF(NEW.app_blocks, '') IS NOT NULL
     OR NULLIF(NEW.app_unfurls, '') IS NOT NULL OR NULLIF(NEW.metadata, '') IS NOT NULL
     OR NULLIF(NEW.card, '') IS NOT NULL OR NEW.home_post_id IS NOT NULL OR NEW.apub THEN
    RAISE EXCEPTION 'messages in an encrypted channel cannot carry plaintext fields' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER messages_e2ee_reject_plaintext BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW EXECUTE FUNCTION e2ee_reject_plaintext_messages();

-- 暗号化チャンネルには App（Bot）と Webhook のユーザーを入れない。
CREATE FUNCTION e2ee_reject_bot_members() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM users WHERE id = NEW.user_id AND is_bot)
     AND EXISTS (SELECT 1 FROM channels WHERE id = NEW.channel_id AND encrypted) THEN
    RAISE EXCEPTION 'apps cannot join an encrypted channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channel_user_e2ee_reject_bots BEFORE INSERT OR UPDATE ON channel_user
  FOR EACH ROW EXECUTE FUNCTION e2ee_reject_bot_members();

-- 暗号化チャンネルのメッセージは App のイベントにしない。
CREATE OR REPLACE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record;
BEGIN
 -- 暗号化チャンネルのメッセージは App のイベントにしない（本文の解析による app.mentioned と link.shared も作らない）。
 IF EXISTS(SELECT 1 FROM channels WHERE id=(CASE WHEN TG_OP='DELETE' THEN OLD.channel_id ELSE NEW.channel_id END) AND encrypted) THEN RETURN NULL; END IF;
 row_data:=(CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END)-'display_data';
 old_data:=(CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END)-'display_data';
 data:=row_data||jsonb_build_object('previous',old_data);
 IF TG_OP='INSERT' THEN
  PERFORM app_emit('message.created',data,NEW.user_id,NEW.channel_id);
  IF NEW.parent_id IS NOT NULL THEN PERFORM app_emit('message.replied',jsonb_build_object('parent_id',NEW.parent_id,'reply',row_data),NEW.user_id,NEW.channel_id);END IF;
  IF NEW.broadcast THEN PERFORM app_emit('message.thread_broadcast',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.message_kind='emote' THEN PERFORM app_emit('message.emote',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.metadata IS NOT NULL THEN PERFORM app_emit('message.metadata_posted',jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb),NEW.user_id,NEW.channel_id);END IF;
  FOR installation IN SELECT i.*,a.bot_user_id,u.user_name FROM app_installations i JOIN apps a ON a.id=i.app_id JOIN users u ON u.id=a.bot_user_id WHERE i.active AND a.status='installed'
  LOOP
   IF is_mention(NEW.content,installation.user_name) THEN PERFORM app_emit('app.mentioned',data,NEW.user_id,NEW.channel_id,NULL,installation.id);END IF;
   IF EXISTS(SELECT 1 FROM jsonb_array_elements_text(COALESCE(installation.manifest::jsonb->'link_domains','[]')) domain WHERE NEW.content ~ ('https://' || replace(domain.value,'.','[.]') || '([/:?#[:space:]]|$)')) THEN
    PERFORM app_emit('link.shared',data,NEW.user_id,NEW.channel_id,NULL,installation.id);
   END IF;
  END LOOP;
 ELSIF TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND old_data->>'deleted_at' IS NULL THEN
  PERFORM app_emit('message.deleted',data,row_data->>'user_id',row_data->>'channel_id');
  IF old_data->>'metadata' IS NOT NULL THEN PERFORM app_emit('message.metadata_deleted',jsonb_build_object('id',OLD.id,'previous_metadata',OLD.metadata::jsonb),OLD.user_id,OLD.channel_id);END IF;
 ELSIF (row_data-'thread_archive_at'-'thread_activity_at') IS DISTINCT FROM (old_data-'thread_archive_at'-'thread_activity_at') THEN
  PERFORM app_emit('message.updated',data,NEW.user_id,NEW.channel_id);
  IF NEW.metadata IS DISTINCT FROM OLD.metadata THEN
   PERFORM app_emit(CASE WHEN NEW.metadata IS NULL THEN 'message.metadata_deleted' WHEN OLD.metadata IS NULL THEN 'message.metadata_posted' ELSE 'message.metadata_updated' END,
    jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb,'previous_metadata',OLD.metadata::jsonb),NEW.user_id,NEW.channel_id);
  END IF;
 END IF;
 RETURN NULL;
END $$;

-- 暗号化チャンネルは Home/LTL に同期しない（暗号化チャンネルは非公開なので今も対象外だが、明示する）。
CREATE OR REPLACE FUNCTION sync_channel_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET parent_id=COALESCE(NEW.parent_id,parent_id),content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at,parent_id) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC',COALESCE(NEW.parent_id,parent_id));
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND NOT c.encrypted AND c.deleted_at IS NULL
      AND (channel_matches_ltl(c.channel_name,s.ltl_channel_pattern) OR EXISTS(
        SELECT 1 FROM home_posts parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL
      ))
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at,parent_id,visibility,reply_user_id)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC',NEW.parent_id,
        COALESCE((SELECT visibility FROM home_posts WHERE id=NEW.parent_id),'local'),
        (SELECT user_id FROM home_posts WHERE id=NEW.parent_id)) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;
