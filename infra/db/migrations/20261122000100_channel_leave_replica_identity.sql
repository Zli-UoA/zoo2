-- channel_leave を Electric で配信するため、UPDATE（再退出による left_at の上書き）と DELETE（CASCADE）を
-- 正しく同期できるよう REPLICA IDENTITY FULL にする（docs/electric-sync-requirements.md）。
ALTER TABLE channel_leave REPLICA IDENTITY FULL;
