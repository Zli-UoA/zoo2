-- Slack 取り込みの再開位置と、デプロイによる中断の記録。
-- import_progress: チャンネルごとに、取り込みが終わった最後の日付ファイル名 (JSON。{"C123": "general/2020-01-02.json"})。
--   日付ファイルは名前の昇順で処理するので、これ以前のファイルは、再開時に飛ばす。
--   ジョブの行ごと (再取り込みは別の行) に持つので、行を作り直せば空から始まる。
-- handoffs: ワーカーの終了 (SIGTERM) で、ジョブを手放した回数。attempts は消費しない。
ALTER TABLE slack_import_jobs
  ADD COLUMN import_progress text,
  ADD COLUMN handoffs integer NOT NULL DEFAULT 0;
