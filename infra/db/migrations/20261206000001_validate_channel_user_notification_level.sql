-- atlas:txmode none

-- 20261206000000 で NOT VALID 追加した CHECK を、ACCESS EXCLUSIVE を取らずに検証する。
-- VALIDATE CONSTRAINT は ShareUpdateExclusiveLock だけを取るため、旧バックエンドの
-- 読み書きを止めずに既存行を走査できる。トランザクションの外で実行するため txmode none。
ALTER TABLE channel_user VALIDATE CONSTRAINT channel_user_notification_level_check;
