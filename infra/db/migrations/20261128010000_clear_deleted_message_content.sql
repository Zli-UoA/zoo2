-- 論理削除したメッセージの本文と付随データを空にする。行は墓石として残るので、Electric の shape から
-- 消える動き（move-out）は変わらない。関数とトリガーの追加・置き換えだけなので、適用前のバックエンドが
-- 動いたままでも壊れない。
CREATE OR REPLACE FUNCTION refresh_message_display(target text) RETURNS void LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  -- Serialize writers for this read model before taking the aggregate snapshot.
  PERFORM 1 FROM messages WHERE id=target FOR NO KEY UPDATE;
  -- 論理削除した行は、投影した内容（投稿者、添付、リアクション）も残さない。
  UPDATE messages m SET display_data=CASE WHEN m.deleted_at IS NULL THEN v.display_data ELSE '{}'::jsonb END
    FROM message_display v
    WHERE m.id=target AND v.id=m.id
      AND m.display_data IS DISTINCT FROM CASE WHEN m.deleted_at IS NULL THEN v.display_data ELSE '{}'::jsonb END;
END;
$$;

CREATE FUNCTION messages_clear_deleted_content() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.content := '';
  NEW.card := NULL;
  NEW.metadata := NULL;
  NEW.display_data := '{}'::jsonb;
  NEW.app_blocks := NULL;
  NEW.app_attachments := NULL;
  NEW.app_unfurls := NULL;
  RETURN NEW;
END $$;
CREATE TRIGGER messages_clear_deleted_content BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL AND (
    NEW.content <> '' OR NEW.card IS NOT NULL OR NEW.metadata IS NOT NULL OR NEW.display_data <> '{}'::jsonb
    OR NEW.app_blocks IS NOT NULL OR NEW.app_attachments IS NOT NULL OR NEW.app_unfurls IS NOT NULL))
  EXECUTE FUNCTION messages_clear_deleted_content();

-- 暗号化チャンネルの平文の検査を、本文が空になった削除済みの行に合わせて直す。削除済みの行の Home との紐付けは
-- 認める（Home との同期が書き戻す）。別のチャンネルから暗号文のない行を移すことと、削除済みの行の復活は認めない。
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
     OR NULLIF(NEW.card, '') IS NOT NULL
     -- Home との同期（sync_home_post）は、削除済みの行を INSERT ... ON CONFLICT で書き戻す。削除済みの行は本文が空なので、
     -- Home との紐付けだけは認める。
     OR (NEW.deleted_at IS NULL AND (NEW.home_post_id IS NOT NULL OR NEW.apub)) THEN
    RAISE EXCEPTION 'messages in an encrypted channel cannot carry plaintext fields' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- 適用済みの削除済みメッセージも空にする。内容の変更を App のイベント（message.updated）として
-- 配らないよう、このトランザクションの間だけ app_emit を止める。
SELECT set_config('zoo.suppress_app_events', 'on', true);
UPDATE messages SET deleted_at = deleted_at
  WHERE deleted_at IS NOT NULL AND (
    content <> '' OR card IS NOT NULL OR metadata IS NOT NULL OR display_data <> '{}'::jsonb
    OR app_blocks IS NOT NULL OR app_attachments IS NOT NULL OR app_unfurls IS NOT NULL);
SELECT set_config('zoo.suppress_app_events', 'off', true);
