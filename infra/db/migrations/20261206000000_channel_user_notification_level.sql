-- チャンネルごとの通知レベル ('all' | 'mentions' | 'mute')。
-- NOT NULL DEFAULT で追加するので既存行は 'mentions' に移行済みになり、バックフィルは不要。
-- 旧バックエンドはこの列を読まないため、適用前に動いているアプリも壊れない。
ALTER TABLE channel_user ADD COLUMN notification_level text NOT NULL DEFAULT 'mentions'
  CHECK (notification_level IN ('all', 'mentions', 'mute'));
