-- E2EE: Bot を端末として暗号化チャンネルに参加させる（#366。仕様は docs/e2ee.md の「Bot の端末」）。
-- チャンネルのメンバーが明示的に許可した Bot だけを、暗号化チャンネルのメンバーにする。許可は Bot が
-- チャンネルから外れると消える（channel_user の行の削除のトリガー）。
CREATE TABLE e2ee_bot_grants (
  channel_id text NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  bot_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  granted_by text REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (channel_id, bot_user_id)
);
CREATE INDEX e2ee_bot_grants_bot ON e2ee_bot_grants (bot_user_id);

-- 暗号化チャンネルには、許可のない App（Bot）と Webhook のユーザーを入れない。
CREATE OR REPLACE FUNCTION e2ee_reject_bot_members() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM users WHERE id = NEW.user_id AND is_bot)
     AND EXISTS (SELECT 1 FROM channels WHERE id = NEW.channel_id AND encrypted)
     AND NOT EXISTS (SELECT 1 FROM e2ee_bot_grants WHERE channel_id = NEW.channel_id AND bot_user_id = NEW.user_id) THEN
    RAISE EXCEPTION 'apps cannot join an encrypted channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- Bot がチャンネルから外れたら（退出、キック、App の管理画面からの削除のどれでも）、許可も消す。
-- もう一度入れるには、メンバーがもう一度許可する。
CREATE FUNCTION e2ee_revoke_bot_grant() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  DELETE FROM e2ee_bot_grants WHERE channel_id = OLD.channel_id AND bot_user_id = OLD.user_id;
  RETURN NULL;
END $$;
CREATE TRIGGER channel_user_e2ee_revoke_bot_grant AFTER DELETE ON channel_user
  FOR EACH ROW EXECUTE FUNCTION e2ee_revoke_bot_grant();

-- 暗号化チャンネルのメッセージは、許可のある Bot の App（e2ee:device を持つインストール）にだけ、暗号文と
-- 照合に使う列だけのイベントにする（本文の解析による app.mentioned と link.shared は作らない）。
CREATE OR REPLACE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record; channel_ref text; encrypted_event text;
BEGIN
 channel_ref:=(CASE WHEN TG_OP='DELETE' THEN OLD.channel_id ELSE NEW.channel_id END);
 IF EXISTS(SELECT 1 FROM channels WHERE id=channel_ref AND encrypted) THEN
  row_data:=(CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END);
  old_data:=(CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END);
  encrypted_event:=(CASE
   WHEN TG_OP='INSERT' THEN 'message.created'
   WHEN TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND old_data->>'deleted_at' IS NULL THEN 'message.deleted'
   WHEN (row_data->'ciphertext',row_data->'revision',row_data->'epoch') IS DISTINCT FROM (old_data->'ciphertext',old_data->'revision',old_data->'epoch') THEN 'message.updated'
   ELSE NULL END);
  IF encrypted_event IS NULL THEN RETURN NULL; END IF;
  data:=jsonb_build_object('id',row_data->'id','channel_id',row_data->'channel_id','parent_id',row_data->'parent_id',
   'user_id',row_data->'user_id','message_kind',row_data->'message_kind','epoch',row_data->'epoch','revision',row_data->'revision',
   'ciphertext',CASE WHEN encrypted_event='message.deleted' THEN 'null'::jsonb ELSE row_data->'ciphertext' END,'encrypted',true,
   'created_at',row_data->'created_at','updated_at',row_data->'updated_at','deleted_at',row_data->'deleted_at');
  FOR installation IN SELECT i.id FROM e2ee_bot_grants g JOIN apps a ON a.bot_user_id=g.bot_user_id JOIN app_installations i ON i.app_id=a.id
   WHERE g.channel_id=channel_ref AND i.active AND a.status='installed' AND i.scopes::jsonb ? 'e2ee:device'
  LOOP
   PERFORM app_emit(encrypted_event,data,row_data->>'user_id',channel_ref,NULL,installation.id);
  END LOOP;
  RETURN NULL;
 END IF;
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
