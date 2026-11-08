-- zoo-dispatcher 専用の DB ロール。パスワードは設定しない（NOLOGIN）。
-- インフラ側が `ALTER ROLE zoo_dispatcher LOGIN PASSWORD '...'` で有効化する。
-- ロールはクラスタ共通なので、既にあれば作らない。
-- 配信関数（app_emit / app_event_access など）は SECURITY DEFINER ではないため、
-- 配信に必要な表の権限をこのロール自身が持つ。
-- 読めないもの: messages（本文の表）、トークンのハッシュ、ユーザー名・チャンネル名。
-- 読めるもの（送信に必要）: 配信キューの payload（イベント本文を含む）と、全アプリの署名鍵。
-- このロールの接続情報が漏れると、キュー内のイベントの中身と署名鍵が漏れる。
DO $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='zoo_dispatcher') THEN
  CREATE ROLE zoo_dispatcher NOLOGIN;
 END IF;
END $$;

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

-- 履歴・期限切れデータの掃除（cleanup）。
GRANT SELECT, DELETE ON app_rate_limits, app_socket_tickets, app_ui, app_responses, app_oauth_codes, app_socket_sessions, app_socket_presence_limits TO zoo_dispatcher;
