-- notification_level の変更は Home の表示内容を変えないため、全員の Home を
-- refresh する通知を発火させない。INSERT / DELETE と Home に効く列の UPDATE は
-- 従来どおり refresh する。旧バックエンドは trigger の発火条件を読まないため、
-- 適用前のアプリもそのまま動く。
DROP TRIGGER IF EXISTS channel_user_home_live ON channel_user;
CREATE TRIGGER channel_user_home_live AFTER INSERT OR UPDATE OF user_id, channel_id, joined_at OR DELETE ON channel_user FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
