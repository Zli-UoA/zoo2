-- Slack の Events API (#293) が知らせた「確かめ直すメッセージ」。編集・削除・リアクションの変更・範囲外のスレッドの返信は、
-- 差分同期の履歴では取れないので、同期が Slack API でそのメッセージ 1 件を取り直して、Zoo を API の状態に合わせる。
-- イベントの本文は保存しない (ID と ts、リアクションの名前だけ)。受信口が書き、同期のワーカーが処理して消す。
-- 新しいテーブルの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
CREATE TABLE slack_message_hints (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  message_ts text NOT NULL,
  kind text NOT NULL,
  slack_user_id text NOT NULL DEFAULT '',
  reaction text NOT NULL DEFAULT '',
  thread_ts text NOT NULL DEFAULT '',
  attempts integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id, message_ts, kind, slack_user_id, reaction),
  CONSTRAINT slack_message_hints_kind_check CHECK (kind IN ('changed', 'deleted', 'reaction', 'reply')),
  CONSTRAINT slack_message_hints_channel_fkey
    FOREIGN KEY (team_id, slack_channel_id) REFERENCES slack_channels (team_id, slack_channel_id)
    ON UPDATE CASCADE ON DELETE CASCADE
);
