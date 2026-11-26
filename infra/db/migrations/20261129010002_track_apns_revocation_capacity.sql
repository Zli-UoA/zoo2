DO $$
BEGIN
  -- 容量表を導入済みのプレビューでは、補正済みの解除時刻と集計値を維持する。
  IF to_regclass('apns_revocation_capacity') IS NOT NULL THEN
    RETURN;
  END IF;
  -- 初期集計とトリガーの作成が終わるまで、このDDLのロックが旧版による変更を止める。
  ALTER TABLE apns_revocations ADD COLUMN invalidated_at timestamp(3);

  CREATE TABLE apns_revocation_capacity (
    id boolean PRIMARY KEY CHECK (id),
    records bigint NOT NULL CHECK (records >= 0)
  );
  INSERT INTO apns_revocation_capacity (id, records) SELECT true, count(*) FROM apns_revocations;
  -- 旧版の時刻にはタイムゾーンが残らないため、移行時点から11分間だけ全件を保護する。
  UPDATE apns_revocations SET revoked_at = (SELECT clock_timestamp() AT TIME ZONE 'UTC');
END;
$$;

CREATE OR REPLACE FUNCTION update_apns_revocation_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE changed bigint;
BEGIN
  IF TG_OP = 'TRUNCATE' THEN
    UPDATE apns_revocation_capacity SET records = 0 WHERE id;
  ELSE
    SELECT count(*) INTO changed FROM affected_revocations;
    IF changed > 0 THEN
      UPDATE apns_revocation_capacity
      SET records = records + CASE WHEN TG_OP = 'INSERT' THEN changed ELSE -changed END
      WHERE id;
    END IF;
  END IF;
  RETURN NULL;
END;
$$;
CREATE OR REPLACE TRIGGER apns_revocations_after_insert AFTER INSERT ON apns_revocations
  REFERENCING NEW TABLE AS affected_revocations
  FOR EACH STATEMENT EXECUTE FUNCTION update_apns_revocation_capacity();
CREATE OR REPLACE TRIGGER apns_revocations_after_delete AFTER DELETE ON apns_revocations
  REFERENCING OLD TABLE AS affected_revocations
  FOR EACH STATEMENT EXECUTE FUNCTION update_apns_revocation_capacity();
CREATE OR REPLACE TRIGGER apns_revocations_after_truncate AFTER TRUNCATE ON apns_revocations
  FOR EACH STATEMENT EXECUTE FUNCTION update_apns_revocation_capacity();
