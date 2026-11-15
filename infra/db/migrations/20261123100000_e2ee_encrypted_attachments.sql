-- E2EE: 暗号化した添付（#364。仕様は docs/e2ee.md の「暗号化した添付」）。
-- 暗号化チャンネルの attachments には、クライアントで暗号化した添付だけを置く。本当のファイル名、MIME、
-- 画像の大きさは Megolm で暗号化した本文に入るので、行には決まったファイル名と Content-Type だけを許し、
-- 画像の情報（width、height、blurhash）を持たせない。保存先も、presign が作る attachments/<channel_id>/ に限る
-- （Home の画像などの平文のオブジェクトを指させない）。API 以外の経路で書き込まれても止める最後の防御。
CREATE FUNCTION e2ee_reject_plaintext_attachments() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM channels WHERE id = NEW.channel_id AND encrypted) THEN
    RETURN NEW;
  END IF;
  IF NEW.file_name <> 'encrypted.bin' OR NEW.content_type <> 'application/octet-stream'
     OR NEW.width IS NOT NULL OR NEW.height IS NOT NULL OR NEW.blurhash IS NOT NULL
     OR left(NEW.storage_key, length('attachments/' || NEW.channel_id || '/')) <> 'attachments/' || NEW.channel_id || '/' THEN
    RAISE EXCEPTION 'attachments in an encrypted channel must be encrypted on the client' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER attachments_e2ee_reject_plaintext BEFORE INSERT OR UPDATE ON attachments
  FOR EACH ROW EXECUTE FUNCTION e2ee_reject_plaintext_attachments();
