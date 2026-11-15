-- E2EE: 既存の非公開チャンネルと DM を暗号化に切り替える（#367。仕様は docs/e2ee.md の「既存のチャンネルの暗号化」）。
-- encrypted は false から true にだけ変えられる（true から false は今までどおり拒否する）。切り替えは
-- POST /api/channels/{id}/encrypt が、チャンネルの行を FOR UPDATE でロックして行う。平文を書く経路（下のトリガーと、
-- API の平文の経路）は、チャンネルの行を FOR KEY SHARE で読んでから encrypted を判定するので、切り替えと直列になる
-- （FOR KEY SHARE は FOR UPDATE とだけ衝突し、外部キーの検査と同じ強さなので、ほかの書き込みを待たせない）。
-- 切り替えるときは、許可のない App（Bot）と Webhook のユーザーがメンバーにいないこと、Incoming Webhook が
-- ないことを求める（API の 409 E2EE_CONVERSION_BLOCKED の最後の防御）。
CREATE OR REPLACE FUNCTION e2ee_protect_channel_encryption() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.encrypted THEN
    RAISE EXCEPTION 'channels.encrypted cannot be changed' USING ERRCODE = '23514';
  END IF;
  IF EXISTS (SELECT 1 FROM channel_user cu JOIN users u ON u.id = cu.user_id
               WHERE cu.channel_id = NEW.id AND u.is_bot
                 AND NOT EXISTS (SELECT 1 FROM e2ee_bot_grants g
                                   WHERE g.channel_id = cu.channel_id AND g.bot_user_id = cu.user_id))
     OR EXISTS (SELECT 1 FROM channel_webhooks WHERE channel_id = NEW.id) THEN
    RAISE EXCEPTION 'remove apps and webhooks before encrypting the channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- 暗号化チャンネルの messages に平文が入らないようにする（#358）。#367 からは、切り替える前の平文の行
-- （epoch と ciphertext のない行）が残る。その行は、本文と平文の列をそのままにするか消すだけの更新
-- （リアクションの表示、論理削除、投稿者の付け替えなど）を認め、新しい平文を足す更新と、暗号文への書き換えは拒否する。
-- 新しい行は、今までどおり暗号文だけを認める。
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
       OR (NEW.apub AND NOT OLD.apub) THEN
      RAISE EXCEPTION 'messages written before the channel was encrypted cannot be changed to carry new content' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
  END IF;
  -- 削除済みの行も本文は空にする（#367）。別のチャンネルから削除済みの平文の行を移して、本文を持ち込ませない。
  -- 暗号化チャンネルで送った行は、もともと本文が空なので、削除しても違反しない。
  IF NEW.content <> '' OR (NEW.deleted_at IS NULL AND NEW.ciphertext IS NULL) THEN
    RAISE EXCEPTION 'messages in an encrypted channel require ciphertext and empty content' USING ERRCODE = '23514';
  END IF;
  IF NULLIF(NEW.app_attachments, '') IS NOT NULL OR NULLIF(NEW.app_blocks, '') IS NOT NULL
     OR NULLIF(NEW.app_unfurls, '') IS NOT NULL OR NULLIF(NEW.metadata, '') IS NOT NULL
     OR NULLIF(NEW.card, '') IS NOT NULL OR NEW.home_post_id IS NOT NULL OR NEW.apub THEN
    RAISE EXCEPTION 'messages in an encrypted channel cannot carry plaintext fields' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- 暗号化チャンネルの attachments には、クライアントで暗号化した添付だけを置く（#364）。#367 からは、切り替える前の
-- 平文の添付が残る。その行は、中身、保存先、紐付けを変えない更新（センシティブの印など）だけを認める。
-- 紐付けていない平文の添付を、切り替えたあとのメッセージに紐付けることはできない。
CREATE OR REPLACE FUNCTION e2ee_reject_plaintext_attachments() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  enc boolean;
BEGIN
  SELECT encrypted INTO enc FROM channels WHERE id = NEW.channel_id FOR KEY SHARE;
  IF NOT COALESCE(enc, false) THEN
    RETURN NEW;
  END IF;
  IF TG_OP = 'UPDATE'
     AND (OLD.file_name <> 'encrypted.bin' OR OLD.content_type <> 'application/octet-stream'
          OR OLD.width IS NOT NULL OR OLD.height IS NOT NULL OR OLD.blurhash IS NOT NULL
          OR left(OLD.storage_key, length('attachments/' || OLD.channel_id || '/')) <> 'attachments/' || OLD.channel_id || '/') THEN
    IF (NEW.file_name, NEW.content_type, NEW.width, NEW.height, NEW.blurhash, NEW.storage_key, NEW.message_id, NEW.channel_id)
       IS NOT DISTINCT FROM (OLD.file_name, OLD.content_type, OLD.width, OLD.height, OLD.blurhash, OLD.storage_key, OLD.message_id, OLD.channel_id) THEN
      RETURN NEW;
    END IF;
    RAISE EXCEPTION 'attachments uploaded before the channel was encrypted cannot be changed' USING ERRCODE = '23514';
  END IF;
  IF NEW.file_name <> 'encrypted.bin' OR NEW.content_type <> 'application/octet-stream'
     OR NEW.width IS NOT NULL OR NEW.height IS NOT NULL OR NEW.blurhash IS NOT NULL
     OR left(NEW.storage_key, length('attachments/' || NEW.channel_id || '/')) <> 'attachments/' || NEW.channel_id || '/' THEN
    RAISE EXCEPTION 'attachments in an encrypted channel must be encrypted on the client' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- 暗号化チャンネルには、許可のない App（Bot）と Webhook のユーザーを入れない（#366）。切り替えと直列にするため、
-- チャンネルの行を FOR KEY SHARE で読んでから判定する。
CREATE OR REPLACE FUNCTION e2ee_reject_bot_members() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  enc boolean;
BEGIN
  IF NOT EXISTS (SELECT 1 FROM users WHERE id = NEW.user_id AND is_bot) THEN
    RETURN NEW;
  END IF;
  SELECT encrypted INTO enc FROM channels WHERE id = NEW.channel_id FOR KEY SHARE;
  IF COALESCE(enc, false)
     AND NOT EXISTS (SELECT 1 FROM e2ee_bot_grants WHERE channel_id = NEW.channel_id AND bot_user_id = NEW.user_id) THEN
    RAISE EXCEPTION 'apps cannot join an encrypted channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

-- 暗号化チャンネルには Incoming Webhook を作らない（#358 の API の拒否の最後の防御）。切り替えと直列にする。
CREATE FUNCTION e2ee_reject_channel_webhooks() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  enc boolean;
BEGIN
  SELECT encrypted INTO enc FROM channels WHERE id = NEW.channel_id FOR KEY SHARE;
  IF COALESCE(enc, false) THEN
    RAISE EXCEPTION 'webhooks cannot be added to an encrypted channel' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channel_webhooks_e2ee_reject BEFORE INSERT OR UPDATE OF channel_id ON channel_webhooks
  FOR EACH ROW EXECUTE FUNCTION e2ee_reject_channel_webhooks();
