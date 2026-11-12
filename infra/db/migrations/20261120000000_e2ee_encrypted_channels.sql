-- E2EE: 暗号化チャンネル（#357。仕様は docs/e2ee.md の「DB」と「membership_epoch と送信の順序」）。
-- channels.encrypted と CHECK channels_encrypted_private は #354 で足した。
ALTER TABLE channels ADD COLUMN membership_epoch bigint NOT NULL DEFAULT 0;

ALTER TABLE messages ADD COLUMN ciphertext jsonb;
ALTER TABLE messages ADD COLUMN epoch bigint;
ALTER TABLE messages ADD COLUMN revision integer NOT NULL DEFAULT 0;
ALTER TABLE messages ADD CONSTRAINT messages_ciphertext_shape
  CHECK (ciphertext IS NULL OR (content = '' AND epoch IS NOT NULL));

-- encrypted は作成時だけ指定できる（両方向の変更を拒否する）。暗号化チャンネルを公開にする変更は
-- CHECK channels_encrypted_private が拒否する。
CREATE FUNCTION e2ee_protect_channel_encryption() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'channels.encrypted cannot be changed' USING ERRCODE = '23514';
END $$;
CREATE TRIGGER channels_encrypted_immutable BEFORE UPDATE ON channels
  FOR EACH ROW WHEN (OLD.encrypted IS DISTINCT FROM NEW.encrypted)
  EXECUTE FUNCTION e2ee_protect_channel_encryption();

-- 論理削除した行（DELETE /api/messages/{id} と履歴の消去）の暗号文は残さない。
CREATE FUNCTION e2ee_clear_deleted_ciphertext() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.ciphertext := NULL;
  RETURN NEW;
END $$;
CREATE TRIGGER messages_clear_deleted_ciphertext BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL AND NEW.ciphertext IS NOT NULL)
  EXECUTE FUNCTION e2ee_clear_deleted_ciphertext();

-- そのユーザーが参加しているすべての暗号化チャンネル（削除済みを除く）の membership_epoch を 1 増やす。
-- 端末の削除、アカウントの無効化、クロス署名鍵の変更、鍵バックアップの版の作成で使う。
-- 行は id の昇順にまとめてロックし、ロックした行だけを更新する（並行した一括更新と行き詰まらない）。
CREATE FUNCTION e2ee_bump_membership_epochs(target_user text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  ids text[];
BEGIN
  SELECT array_agg(id) INTO ids FROM (
    SELECT id FROM channels
      WHERE encrypted AND deleted_at IS NULL
        AND id IN (SELECT channel_id FROM channel_user WHERE user_id = target_user)
      ORDER BY id FOR NO KEY UPDATE
  ) locked;
  IF ids IS NOT NULL THEN
    UPDATE channels SET membership_epoch = membership_epoch + 1
      WHERE id = ANY (ids) AND encrypted AND deleted_at IS NULL;
  END IF;
END $$;

-- アカウントの無効化でも、参加しているすべての暗号化チャンネルの membership_epoch を増やす。
-- ロックの順（users の行、端末単位のロック、channels の行、e2ee_devices の行）に合わせ、端末単位のロックのあと、
-- e2ee_devices の更新の前に増やす。
CREATE OR REPLACE FUNCTION e2ee_delete_devices_on_deactivate() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  d text;
BEGIN
  -- 鍵の行も端末ごとに device_id の昇順で消す。keys/claim は宛先を (user_id, device_id) の昇順で
  -- 払い出すので、同じ順で行ロックを取り、並行した claim と行き詰まらない。
  FOR d IN SELECT device_id FROM e2ee_devices
      WHERE user_id = NEW.id AND deleted_at IS NULL ORDER BY device_id COLLATE "C" LOOP
    PERFORM pg_advisory_xact_lock(2, hashtext(NEW.id || ':' || d));
    DELETE FROM e2ee_key_signatures
      WHERE signer_user_id = NEW.id AND target_user_id = NEW.id AND target_key_id = d;
    DELETE FROM e2ee_one_time_keys WHERE user_id = NEW.id AND device_id = d;
    DELETE FROM e2ee_fallback_keys WHERE user_id = NEW.id AND device_id = d;
    -- 未受信の to-device メッセージも消す（#355）。to-device の送信は同じ端末単位のロックのあとに
    -- deleted_at を読み直すので、このあとで削除済みの端末に入り直らない。
    DELETE FROM e2ee_to_device_messages WHERE recipient_user_id = NEW.id AND recipient_device_id = d;
  END LOOP;
  PERFORM e2ee_bump_membership_epochs(NEW.id);
  UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = NEW.id AND deleted_at IS NULL;
  RETURN NULL;
END $$;

-- membership_epoch だけの変更では、通知の表示、App のイベント、Home の同期を作り直さない。
CREATE OR REPLACE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF (to_jsonb(OLD) - 'membership_epoch') IS NOT DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch') THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('channel',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION app_channel_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;channel text:=row_data->>'id';actor text:=COALESCE(NULLIF(current_setting('zoo.actor_id',true),''),row_data->>'created_user_id');
BEGIN
 -- membership_epoch だけの変更（暗号化チャンネルのメンバーや端末の変化）は、チャンネルのイベントにしない。
 IF TG_OP='UPDATE' AND (to_jsonb(OLD) - 'membership_epoch') IS NOT DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch') THEN RETURN NULL; END IF;
 IF TG_OP='INSERT' AND NOT NEW.is_private AND NOT NEW.is_dm THEN
  PERFORM app_emit(CASE WHEN NEW.is_dm THEN 'conversation.created' ELSE 'channel.created' END,row_data,actor,channel);
 ELSIF TG_OP='INSERT' THEN NULL;
 ELSIF TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND to_jsonb(OLD)->>'deleted_at' IS NULL THEN
  PERFORM app_emit('channel.deleted',row_data,actor,channel);
 ELSE
  IF OLD.archive_at IS DISTINCT FROM NEW.archive_at THEN PERFORM app_emit(CASE WHEN NEW.archive_at IS NULL THEN 'channel.unarchived' ELSE 'channel.archived' END,row_data,actor,channel);END IF;
  IF OLD.channel_name<>NEW.channel_name THEN PERFORM app_emit('channel.renamed',row_data||jsonb_build_object('previous_name',OLD.channel_name),actor,channel);END IF;
  IF OLD.description IS DISTINCT FROM NEW.description THEN PERFORM app_emit('channel.purpose_changed',row_data,actor,channel);END IF;
  IF OLD.topic IS DISTINCT FROM NEW.topic THEN PERFORM app_emit('channel.topic_changed',row_data,actor,channel);END IF;
  IF OLD.is_private<>NEW.is_private THEN PERFORM app_emit('channel.privacy_changed',row_data||jsonb_build_object('previous_is_private',OLD.is_private),actor,channel);END IF;
  IF OLD.posting_policy<>NEW.posting_policy THEN PERFORM app_emit('channel.posting_permissions_changed',row_data,actor,channel);END IF;
  IF to_jsonb(OLD) IS DISTINCT FROM row_data THEN PERFORM app_emit('channel.updated',row_data,actor,channel);END IF;
 END IF;
 RETURN NULL;
END $$;

-- 文レベルのトリガーは OLD と NEW を見られないので、UPDATE だけを行レベルのトリガーに分ける。
DROP TRIGGER channels_home_live ON channels;
CREATE TRIGGER channels_home_live AFTER INSERT OR DELETE ON channels FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channels_home_live_update AFTER UPDATE ON channels FOR EACH ROW
  WHEN ((to_jsonb(OLD) - 'membership_epoch') IS DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch'))
  EXECUTE FUNCTION notify_home_timeline();

-- 通知の表示に埋め込むチャンネルは列を明示し、membership_epoch を含めない。
CREATE OR REPLACE VIEW notification_display AS
SELECT n.id, jsonb_build_object(
  'message', CASE WHEN m.deleted_at IS NULL AND m.id IS NOT NULL THEN
    jsonb_build_object('id',m.id,'user_id',m.user_id,'content',left(m.content,281),
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
