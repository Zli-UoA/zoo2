-- Slack チャンネルの対応先 (取り込み先) を付け替えたら、同期の途中経過を捨てる。
-- 途中経過 (ページのカーソル・残りのスレッド・今回の最大 ts) は、古い対応先のためのものなので、
-- 新しい対応先には使えない。カーソル (slack_channels.last_ts) はブリッジの付け替えが NULL にし、
-- ここでは再開位置と実行中の印を戻す。
CREATE FUNCTION reset_slack_channel_sync_on_remap() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE slack_channel_syncs
  SET running=false,
      phase='history',
      run_oldest=NULL,
      run_max_ts=NULL,
      page_cursor=NULL,
      pending_threads='[]',
      swept_at=NULL,
      run_light=false,
      lease_token=NULL,
      lease_until=NULL,
      updated_at=now()
  WHERE team_id=NEW.team_id AND slack_channel_id=NEW.slack_channel_id;
  RETURN NEW;
END $$;

CREATE TRIGGER reset_slack_channel_sync_on_remap
AFTER UPDATE OF channel_id ON slack_channels
FOR EACH ROW WHEN (OLD.channel_id IS DISTINCT FROM NEW.channel_id)
EXECUTE FUNCTION reset_slack_channel_sync_on_remap();
