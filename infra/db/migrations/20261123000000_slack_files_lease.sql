-- 添付の行単位の取得 (ジョブに紐づかない、または終わったジョブの添付) のリース。
-- lease_until は、取得中のリースの期限、または失敗後に次に試す時刻。lease_token は取得中の持ち主 (取得中だけ入る)。
ALTER TABLE slack_files ADD COLUMN lease_until timestamptz, ADD COLUMN lease_token text;
-- 取得待ちの行を、古い順に引くためのインデックス。
CREATE INDEX idx_slack_files_pending_rows ON slack_files (created_at) WHERE status = 'pending';
