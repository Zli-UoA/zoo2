-- Baseline from main before the seek/active-team changes. Not application SQL.
WITH c AS (SELECT t.team_id,t.slack_channel_id FROM slack_channel_syncs t
 JOIN slack_channels sc ON sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id
 JOIN channels ch ON ch.id=sc.channel_id
 WHERE NOT COALESCE(ch.encrypted,false)
 AND (t.available_at<=now() OR (NOT t.running
 AND (t.not_before IS NULL OR t.not_before<=now())
 AND (t.last_synced_at IS NULL OR t.last_synced_at<=now()-interval '30 seconds')
 AND EXISTS(SELECT 1 FROM slack_channel_wakes w WHERE w.team_id=t.team_id
 AND w.slack_channel_id=t.slack_channel_id AND w.first_at<=now()-interval '3 seconds')))
 AND (t.lease_until IS NULL OR t.lease_until<now())
 AND (sc.last_ts IS NOT NULL OR ch.slack_imported_at IS NOT NULL)
 AND NOT EXISTS(SELECT 1 FROM slack_import_jobs j WHERE j.team_id=t.team_id
 AND j.status IN ('analyzing','importing'))
 ORDER BY (NOT t.running AND EXISTS(SELECT 1 FROM slack_channel_wakes w WHERE w.team_id=t.team_id
 AND w.slack_channel_id=t.slack_channel_id AND w.first_at<=now()-interval '3 seconds')) DESC,
 t.available_at,t.team_id,t.slack_channel_id FOR UPDATE OF t SKIP LOCKED LIMIT 1)
 UPDATE slack_channel_syncs t SET lease_until=now()+interval '5 minutes',lease_token=$1,updated_at=now()
 FROM c, slack_channels sc, channels ch
 WHERE t.team_id=c.team_id AND t.slack_channel_id=c.slack_channel_id
 AND sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id AND ch.id=sc.channel_id
 RETURNING json_build_object('team_id',t.team_id,'slack_channel_id',t.slack_channel_id,
 'channel_id',ch.id,'private',ch.is_private,'deleted',ch.deleted_at IS NOT NULL,'encrypted',ch.encrypted,
 'last_ts',sc.last_ts,'synced',t.last_synced_at IS NOT NULL,'token',t.lease_token,'attempts',t.attempts,'running',t.running,
 'phase',t.phase,'run_oldest',t.run_oldest,'run_max_ts',t.run_max_ts,
 'page_cursor',t.page_cursor,'pending_threads',t.pending_threads,
 'known_threads',t.known_threads,'token_user_id',t.token_user_id,
 'swept_recent',(t.swept_at IS NOT NULL AND t.swept_at>now()-interval '5 minutes'),
 'run_light',t.run_light,
 'recheck_due',(t.rechecked_at IS NULL OR t.rechecked_at<now()-interval '30 minutes'))::text
