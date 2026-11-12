-- スレッドの自動アーカイブ状態と活動時刻。どちらも NULL 許容で追加し、旧バックエンドとの互換性を保つ。
ALTER TABLE "messages" ADD COLUMN "thread_archive_at" timestamp(3);
ALTER TABLE "messages" ADD COLUMN "thread_activity_at" timestamp(3);

-- スレッド状態だけの更新を App 向け message.updated イベントとして扱わない。
CREATE OR REPLACE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record;
BEGIN
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

-- 投稿経路によらず、返信と同じトランザクションで活動時刻と再開状態を更新する。
CREATE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  UPDATE messages parent SET
    thread_activity_at=GREATEST(parent.thread_activity_at,NEW.created_at),
    -- INSERT 前にアーカイブされた通常の返信も再開し、過去の投稿の取り込みと区別する。
    thread_archive_at=CASE WHEN NEW.created_at >= parent.thread_archive_at
      OR NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC')
      THEN NULL ELSE parent.thread_archive_at END
    WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_thread_activity_insert AFTER INSERT ON messages
  FOR EACH ROW EXECUTE FUNCTION update_thread_activity();
CREATE TRIGGER messages_thread_activity_parent_update AFTER UPDATE OF parent_id ON messages
  FOR EACH ROW EXECUTE FUNCTION update_thread_activity();
