WITH c AS MATERIALIZED (SELECT t.team_id,t.slack_channel_id FROM /* candidate_from */
 CROSS JOIN LATERAL (SELECT channel_id,last_ts FROM slack_channels sc WHERE sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id OFFSET 0) sc
 CROSS JOIN LATERAL (SELECT encrypted,slack_imported_at FROM channels ch WHERE ch.id=sc.channel_id OFFSET 0) ch
 WHERE /* team_filter */ AND NOT COALESCE(ch.encrypted,false) AND /* readiness */
 AND (t.lease_until IS NULL OR t.lease_until<now())
 AND (sc.last_ts IS NOT NULL OR ch.slack_imported_at IS NOT NULL)
 AND NOT EXISTS(SELECT 1 FROM slack_import_jobs j WHERE j.team_id=t.team_id
 AND j.status IN ('analyzing','importing') OFFSET 0)
 ORDER BY t.available_at,t.team_id,t.slack_channel_id FOR UPDATE OF t SKIP LOCKED LIMIT 1)
 UPDATE slack_channel_syncs t SET lease_until=now()+{LEASE_SQL},lease_token=$1,updated_at=now()
 FROM c
 CROSS JOIN LATERAL (SELECT channel_id,last_ts FROM slack_channels sc WHERE sc.team_id=c.team_id AND sc.slack_channel_id=c.slack_channel_id OFFSET 0) sc
 CROSS JOIN LATERAL (SELECT id,is_private,deleted_at,encrypted FROM channels ch WHERE ch.id=sc.channel_id OFFSET 0) ch
 WHERE t.team_id=c.team_id AND t.slack_channel_id=c.slack_channel_id
 RETURNING json_build_object('team_id',t.team_id,'slack_channel_id',t.slack_channel_id,
 'channel_id',ch.id,'private',ch.is_private,'deleted',ch.deleted_at IS NOT NULL,'encrypted',ch.encrypted,
 'last_ts',sc.last_ts,'synced',t.last_synced_at IS NOT NULL,'token',t.lease_token,'attempts',t.attempts,'running',t.running,
 'phase',t.phase,'run_oldest',t.run_oldest,'run_max_ts',t.run_max_ts,
 'page_cursor',t.page_cursor,'pending_threads',t.pending_threads,
 'known_threads',t.known_threads,'token_user_id',t.token_user_id,
 'swept_recent',(t.swept_at IS NOT NULL AND t.swept_at>now()-interval '5 minutes'),
 'run_light',t.run_light,
 'recheck_due',(t.rechecked_at IS NULL OR t.rechecked_at<now()-interval '30 minutes'))::text
