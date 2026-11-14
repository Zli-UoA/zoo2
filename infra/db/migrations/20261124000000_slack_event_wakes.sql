-- Slack の Events API (#293) の「今すぐ同期して」の合図と、同期の待ち (not_before)。
-- 新しいテーブルと NULL 許容のカラムの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
-- not_before: 429・失敗・保留のあとに、次に同期してよい日時。合図で前倒しするときも、これより前にはしない。
ALTER TABLE slack_channel_syncs ADD COLUMN not_before timestamptz;

-- 受信口は、この表にだけ書く。slack_channel_syncs の行は、同期のトランザクションが長く握る。
CREATE TABLE slack_channel_wakes (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  first_at timestamptz NOT NULL DEFAULT now(),
  last_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id),
  CONSTRAINT slack_channel_wakes_channel_fkey
    FOREIGN KEY (team_id, slack_channel_id) REFERENCES slack_channels (team_id, slack_channel_id)
    ON UPDATE CASCADE ON DELETE CASCADE
);
