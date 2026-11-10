-- atlas:txmode none

-- 滞留（未 ACK・期限内）の集計用インデックス。app_deliveries は書き込みの多い大きな表なので、
-- 書き込みを止めないよう CONCURRENTLY で作る（トランザクションの外で実行するため txmode none）。
-- 途中で失敗して INVALID なインデックスが残った場合に備え、IF NOT EXISTS ではなく、
-- 失敗時は該当インデックスを DROP INDEX CONCURRENTLY してから再適用する。
CREATE INDEX CONCURRENTLY app_deliveries_backlog ON app_deliveries(installation_id,expires_at) INCLUDE (created_at,event_type) WHERE acked_at IS NULL;
CREATE INDEX CONCURRENTLY app_deliveries_queue ON app_deliveries(expires_at) INCLUDE (created_at) WHERE acked_at IS NULL;
