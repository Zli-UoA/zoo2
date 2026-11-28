-- チャンネルごとの通知レベル ('all' | 'mentions' | 'mute')。
-- NOT NULL DEFAULT で追加するので既存行は 'mentions' に移行済みになり、バックフィルは不要。
-- 旧バックエンドはこの列を読まないため、適用前に動いているアプリも壊れない。
-- CHECK を ADD COLUMN と同じ文で付けると既存行の走査・検証のために ACCESS EXCLUSIVE ロックを
-- 長く保持する。列追加と制約追加を分け、制約は NOT VALID で追加して既存行の検証を
-- 20261206000001 の VALIDATE CONSTRAINT に委譲する。
ALTER TABLE channel_user ADD COLUMN notification_level text NOT NULL DEFAULT 'mentions';
ALTER TABLE channel_user ADD CONSTRAINT channel_user_notification_level_check
  CHECK (notification_level IN ('all', 'mentions', 'mute')) NOT VALID;
