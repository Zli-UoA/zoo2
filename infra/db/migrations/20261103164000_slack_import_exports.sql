-- アップロードした Slack エクスポートを残し、取り込みのたびに別のジョブを作る (#295)。
-- エクスポートは source_id が NULL で、解析後は ready のまま残る。取り込みのジョブは source_id にエクスポートを持つ。
ALTER TABLE slack_import_jobs ADD COLUMN source_id text;
ALTER TABLE slack_import_jobs ADD CONSTRAINT slack_import_jobs_source_id_fkey
  FOREIGN KEY (source_id) REFERENCES slack_import_jobs (id)
  ON UPDATE CASCADE ON DELETE RESTRICT;
CREATE INDEX idx_slack_import_jobs_source_id ON slack_import_jobs (source_id);

ALTER TABLE slack_import_jobs DROP CONSTRAINT slack_import_jobs_status_check;
UPDATE slack_import_jobs SET status = 'ready' WHERE status = 'awaiting_mapping';
ALTER TABLE slack_import_jobs ADD CONSTRAINT slack_import_jobs_status_check CHECK (status IN (
  'uploading', 'analyzing', 'ready', 'importing',
  'fetching_files', 'completed', 'failed', 'cancelled'
));
