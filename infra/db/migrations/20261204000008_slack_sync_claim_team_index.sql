-- atlas:txmode none

-- チーム指定の claim が、ほかのチームの期限到来行を走査しないようにする。
-- 全体の claim は slack_channel_syncs_due_order (available_at 先頭) を使い続ける。
-- Create index "slack_channel_syncs_team_due_order" to table: "slack_channel_syncs"
CREATE INDEX CONCURRENTLY "slack_channel_syncs_team_due_order" ON "slack_channel_syncs" ("team_id", "available_at", "slack_channel_id");
