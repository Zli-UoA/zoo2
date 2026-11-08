-- zoo-dispatcher を即時に起こす。HTTP インストール宛ての配信が追加されたときに NOTIFY する。
-- 文単位のトリガーで、同一トランザクション内の通知は 1 通に集約される。WebSocket 宛ての配信
-- （dispatcher の対象外）では通知しない。遷移テーブルで、その文が挿入した行の宛先を見る。
-- 手動再送・バックオフ明け・リース切れは INSERT ではないため、dispatcher は数秒間隔のポーリングも併用する。
CREATE FUNCTION app_delivery_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS (SELECT 1 FROM new_rows n JOIN app_installations i ON i.id=n.installation_id WHERE i.manifest::jsonb->>'transport'='http') THEN
  PERFORM pg_notify('app_delivery','');
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER app_deliveries_notify AFTER INSERT ON app_deliveries REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION app_delivery_notify();
