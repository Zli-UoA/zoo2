-- メッセージ部品（Discord Components V2 形式、docs/apps.md「メッセージ部品」）を入れる列を足す。
-- これまでの card（MessageCard）と app_blocks（旧 Block の一覧）は、この列に移していく。
-- 旧列はまだ消さない（後のマイグレーションで消す）。列の追加と関数の置き換えだけなので、適用前のバックエンドが
-- 動いたままでも壊れない。既存の行の変換は zoo_components_backfill で行う（暗号化チャンネルの行は変換しない）。
ALTER TABLE messages ADD COLUMN components text;
ALTER TABLE app_link_unfurls ADD COLUMN components text;
-- 新しいバックエンドは components だけを書く。旧バックエンドは card を書くので、NOT NULL だけ外す。
ALTER TABLE app_link_unfurls ALTER COLUMN card DROP NOT NULL;

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
  RETURN NEW;
END $$;
DROP TRIGGER messages_clear_deleted_content ON messages;
CREATE TRIGGER messages_clear_deleted_content BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL AND (
    NEW.content <> '' OR NEW.card IS NOT NULL OR NEW.components IS NOT NULL OR NEW.metadata IS NOT NULL
    OR NEW.display_data <> '{}'::jsonb
    OR NEW.app_blocks IS NOT NULL OR NEW.app_attachments IS NOT NULL OR NEW.app_unfurls IS NOT NULL))
  EXECUTE FUNCTION messages_clear_deleted_content();

-- 暗号化チャンネルの平文の検査に components を足す（20261128010000 の本体に、components の 2 か所を足したもの）。
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
     -- Home との同期（sync_home_post）は、削除済みの行を INSERT ... ON CONFLICT で書き戻す。削除済みの行は本文が空なので、
     -- Home との紐付けだけは認める。
     OR (NEW.deleted_at IS NULL AND (NEW.home_post_id IS NOT NULL OR NEW.apub)) THEN
    RAISE EXCEPTION 'messages in an encrypted channel cannot carry plaintext fields' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- external_select の選択肢の問い合わせ（invoke の kind "options"）。アプリの応答を 1 分だけ置く。
ALTER TABLE app_ui DROP CONSTRAINT app_ui_kind_check;
ALTER TABLE app_ui ADD CONSTRAINT app_ui_kind_check CHECK(kind IN ('home','modal','ephemeral','trigger','options'));
INSERT INTO app_event_types VALUES('options.requested',NULL,'app');

-- モーダルのファイル送信（file_upload）。利用者が上げた添付を、送信先のアプリが 1 日だけ取得できるようにする。
CREATE TABLE app_submitted_files (
  attachment_id text NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
  installation_id text NOT NULL REFERENCES app_installations(id) ON DELETE CASCADE,
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (attachment_id, installation_id)
);
CREATE INDEX app_submitted_files_installation ON app_submitted_files(installation_id);
CREATE INDEX app_submitted_files_expiry ON app_submitted_files(expires_at);
-- 期限切れの画面の掃除（delivery.rs）。問い合わせ（options）の行は 1 分で切れて増えるので、全件を読まずに
-- 期限切れの行だけを、どの接続でも同じ順に消す（全件読みでは開始位置が接続ごとに違い、同時の掃除がデッドロックする）
CREATE INDEX app_ui_expiry ON app_ui(expires_at) WHERE expires_at IS NOT NULL;
-- 利用者の画面の一覧（GET /api/apps/{id}/surfaces）。全件を読まず、その利用者の画面だけを更新順に読む。
-- 一覧に出さない trigger・options（短い間だけの行）は入れない（入れると、消えた行の項目が溜まり、主キーで引く問い合わせの計画もこの索引に寄る）
CREATE INDEX app_ui_user ON app_ui(user_id, updated_at) WHERE kind NOT IN ('trigger','options');
