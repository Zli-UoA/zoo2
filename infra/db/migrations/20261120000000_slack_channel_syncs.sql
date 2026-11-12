-- Slack の差分同期 (#292) のキューと再開位置。サーバー専用で、Electric の Shape には載せない。
-- Slack チャンネル (slack_channels) ごとに 1 行。同期の進み具合は slack_channels.last_ts (チャンネル単位で共有するカーソル) で、
-- ここには 1 回の同期の途中経過 (ページのカーソル、残りのスレッド) と、次に実行する日時・リースを持つ。
-- 新しいテーブルの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
CREATE TABLE slack_channel_syncs (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_until timestamptz,
  lease_token text,
  attempts integer NOT NULL DEFAULT 0,
  running boolean NOT NULL DEFAULT false,
  phase text NOT NULL DEFAULT 'history',
  run_oldest text,
  run_max_ts text,
  page_cursor text,
  pending_threads text NOT NULL DEFAULT '[]',
  known_threads text NOT NULL DEFAULT '[]',
  token_user_id text,
  last_synced_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id),
  CONSTRAINT slack_channel_syncs_phase_check CHECK (phase IN ('history', 'threads')),
  CONSTRAINT slack_channel_syncs_channel_fkey
    FOREIGN KEY (team_id, slack_channel_id) REFERENCES slack_channels (team_id, slack_channel_id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_channel_syncs_due ON slack_channel_syncs (available_at);
