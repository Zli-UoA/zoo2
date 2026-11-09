-- 取り込みジョブごとの添付の件数 (pending/failed) と取得対象を引くためのインデックス。
-- pending だけの部分インデックスでは failed の件数を数えられないので置き換える。
DROP INDEX idx_slack_files_pending;
CREATE INDEX idx_slack_files_job_status ON slack_files (job_id, status);
-- 取得した添付ごとの本文更新 (ファイル名の補完・除去) が、投稿の添付だけを引けるようにする。
-- 主キー (team_id, slack_file_id, message_id) では message_id で絞れず、チームの全ファイルをたどってしまう。
CREATE INDEX idx_slack_files_message ON slack_files (team_id, message_id);
