-- ユーザーが自分でチャンネルを退出した日時（「チャンネルを探す」の退出日での並び替え用）。
-- テーブルの追加だけなので、適用前のバックエンドが動いたままでも壊れない（旧バックエンドはこのテーブルを使わない）。
CREATE TABLE channel_leave (
  user_id text NOT NULL,
  channel_id text NOT NULL,
  left_at timestamp(3) NOT NULL,
  CONSTRAINT channel_leave_pkey PRIMARY KEY (user_id, channel_id),
  CONSTRAINT channel_leave_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT channel_leave_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);
