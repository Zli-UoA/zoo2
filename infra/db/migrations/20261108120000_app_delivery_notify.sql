-- zoo-dispatcher を即時に起こす。配信の追加ごとに NOTIFY する（文単位で、同一トランザクション内は 1 通に集約される）。
-- 手動再送・バックオフ明け・リース切れは INSERT ではないため、dispatcher は数秒間隔のポーリングも併用する。
CREATE FUNCTION app_delivery_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM pg_notify('app_delivery','');
 RETURN NULL;
END $$;
CREATE TRIGGER app_deliveries_notify AFTER INSERT ON app_deliveries FOR EACH STATEMENT EXECUTE FUNCTION app_delivery_notify();
