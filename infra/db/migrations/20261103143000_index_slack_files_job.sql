-- 取り込みジョブごとの添付の件数 (pending/failed) と取得対象を引くためのインデックス。
-- pending だけの部分インデックスでは failed の件数を数えられないので置き換える。
DROP INDEX idx_slack_files_pending;
CREATE INDEX idx_slack_files_job_status ON slack_files (job_id, status);
