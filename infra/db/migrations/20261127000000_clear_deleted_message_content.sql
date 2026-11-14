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

-- 適用済みの削除済みメッセージも空にする。内容の変更を App のイベント（message.updated）として
-- 配らないよう、このトランザクションの間だけそのトリガーを止める。
ALTER TABLE messages DISABLE TRIGGER app_message_events;
UPDATE messages SET deleted_at = deleted_at
  WHERE deleted_at IS NOT NULL AND (
    content <> '' OR card IS NOT NULL OR metadata IS NOT NULL OR display_data <> '{}'::jsonb
    OR app_blocks IS NOT NULL OR app_attachments IS NOT NULL OR app_unfurls IS NOT NULL);
ALTER TABLE messages ENABLE TRIGGER app_message_events;
