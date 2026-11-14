-- Slack の Events API (#293) の「今すぐ同期して」の合図と、同期の待ち (not_before)。
-- 新しいテーブルと NULL 許容のカラムの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
-- not_before: 429・失敗・保留のあとに、次に同期してよい日時。合図で前倒しするときも、これより前にはしない。
ALTER TABLE slack_channel_syncs ADD COLUMN not_before timestamptz;
-- 適用の時点で次の実行が先になっている項目は、その日時まで合図で前倒ししない (429 などの待ちの途中で適用されても守る)。
-- 通常の定期の待ち (最大 5 分) の項目も対象になるが、適用後の最初の 1 回の定期の実行まで、合図が待つだけである
UPDATE slack_channel_syncs SET not_before=available_at WHERE available_at>now();

-- 監視中のスレッドを全部取り直す同期 (定期の実行) を、最後に始めた日時。合図で起きる同期は、直近 5 分以内に
-- 取り直していれば、スレッドの総なめを省く (Slack の API の呼び出しを抑える)
ALTER TABLE slack_channel_syncs ADD COLUMN swept_at timestamptz;
-- 実行中の同期が、スレッドの総なめを省く同期 (軽い同期) か。軽い同期は、監視中のスレッドの返信を取らないので、
-- 終わっても同期カーソル (slack_channels.last_ts) を進めない (次の全部取り直しが、返信を `oldest=カーソル` で取れるようにする)
ALTER TABLE slack_channel_syncs ADD COLUMN run_light boolean NOT NULL DEFAULT false;

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
