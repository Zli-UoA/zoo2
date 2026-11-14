-- Slack のメッセージの `ts` と Zoo のメッセージ ID の対応表 (#293 のループ防止。書き込みは #294 の Zoo → Slack のブリッジ)。
-- #294 が Zoo の投稿を Slack に投稿 (`chat.postMessage`) したときの `ts` を、Zoo のメッセージ ID に結び付ける。
-- 差分同期・イベントの反映は、この表を先に見る: `origin='zoo'` (Zoo で作った投稿を Slack に出したもの) の `ts` は、
-- Slack から取り込まず (重複・ループを防ぐ)、編集・削除も反映しない (Zoo が正)。Slack のスレッドの返信や、
-- リアクションは、その `message_id` の投稿に付ける。`origin='slack'` の行は、決定的 ID と違う ID を使いたいときに使える。
-- 新しいテーブルの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
CREATE TABLE slack_message_map (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  ts text NOT NULL,
  message_id text NOT NULL,
  origin text NOT NULL DEFAULT 'zoo',
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id, ts),
  CONSTRAINT slack_message_map_origin_check CHECK (origin IN ('zoo', 'slack')),
  CONSTRAINT slack_message_map_message_id_key UNIQUE (message_id),
  CONSTRAINT slack_message_map_channel_fkey
    FOREIGN KEY (team_id, slack_channel_id) REFERENCES slack_channels (team_id, slack_channel_id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT slack_message_map_message_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);
