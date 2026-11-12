-- zoo_dispatcher の権限を付け直す（zoo2#323）。
-- 20261109010000_app_delivery_role は、ロールがまだ無い本番に適用されたため、付与を飛ばした（NOTICE）。
-- ロールは zoo2-infra の CNPG managed role で作る（Zli-UoA/zoo2-infra#164）。そのあとにこのファイルが
-- 適用されれば、手で psql を流さなくても権限が付く。内容は元のマイグレーションの GRANT と同じで、
-- 何度流しても同じ結果になる。ロールがまだ無ければ、また NOTICE を出して飛ばす（失敗はしない）。
DO $$
BEGIN
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='zoo_dispatcher') THEN
  GRANT USAGE ON SCHEMA public TO zoo_dispatcher;
  -- 配信キュー。UPDATE は配信の進行に使う列だけ。
  GRANT SELECT, INSERT, DELETE ON app_deliveries TO zoo_dispatcher;
  GRANT UPDATE (attempts, first_delivered_at, lease_until, lease_token, available_at, acked_at, last_error) ON app_deliveries TO zoo_dispatcher;
  GRANT SELECT ON app_installations TO zoo_dispatcher;
  GRANT UPDATE (last_http_dispatch_at) ON app_installations TO zoo_dispatcher;
  -- 配信可否の判定（app_event_access / app_event_matches / app_emit）が読む表。
  GRANT SELECT ON app_event_types, app_event_aliases TO zoo_dispatcher;
  GRANT SELECT (id, bot_user_id, status, deleted_at) ON apps TO zoo_dispatcher;
  GRANT SELECT (installation_id, user_id, scopes, revoked_at, expires_at) ON app_credentials TO zoo_dispatcher;
  GRANT SELECT (id, is_bot, is_guest, is_deactivated) ON users TO zoo_dispatcher;
  GRANT SELECT (id, is_private, is_dm, deleted_at) ON channels TO zoo_dispatcher;
  GRANT SELECT (channel_id, user_id) ON channel_user TO zoo_dispatcher;
  -- 配信履歴の 30 日保持は app_deliveries の DELETE だけで足りる。API 側の期限切れ削除は backend が行う。
 ELSE
  RAISE NOTICE 'role zoo_dispatcher does not exist yet; create it (CNPG managed role), then run this file again (psql -f) to grant privileges';
 END IF;
END $$;
