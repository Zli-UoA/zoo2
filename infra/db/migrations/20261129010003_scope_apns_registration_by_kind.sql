-- atlas:txmode none

-- 既存の登録と解除情報を残したまま、通常通知とVoIPの一意性を分ける。
-- 新索引を先に完成させ、構築中も登録・更新・解除を受け付ける。
CREATE UNIQUE INDEX CONCURRENTLY "apns_subscriptions_token_environment_kind_key" ON "apns_subscriptions" (
  "device_token", "environment",
  (id = encode(sha256(('voip:' || environment || ':' || device_token)::bytea), 'hex'))
);

-- 索引構築に失敗した場合は、この文に到達せず元の制約が残る。
-- 再開時にも待機上限を適用し、成功・失敗のどちらでも接続設定を残さない。
DO $$
BEGIN
  SET LOCAL lock_timeout = '2s';
  ALTER TABLE "apns_subscriptions" DROP CONSTRAINT "apns_subscriptions_device_token_environment_key";
END
$$;
