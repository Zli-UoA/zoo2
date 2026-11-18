-- 先行プレビューで同じテーブルが作成済みの場合は、登録と解除記録を引き継ぐ。
CREATE TABLE IF NOT EXISTS "apns_revocations" (
  "id" text NOT NULL,
  "revoked_at" timestamp(3) NOT NULL,
  PRIMARY KEY ("id")
);
CREATE INDEX IF NOT EXISTS "idx_apns_revocations_revoked_at" ON "apns_revocations" ("revoked_at");

CREATE TABLE IF NOT EXISTS "apns_subscriptions" (
  "id" text NOT NULL,
  "user_id" text NOT NULL,
  "device_token" text NOT NULL,
  "environment" text NOT NULL,
  "revocation_hash" text NOT NULL,
  "created_at" timestamp(3) NOT NULL,
  "last_test_at" timestamptz,
  PRIMARY KEY ("id"),
  CONSTRAINT "apns_subscriptions_device_token_environment_key" UNIQUE ("device_token", "environment"),
  CONSTRAINT "apns_subscriptions_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT "apns_subscriptions_environment_check" CHECK (environment = ANY (ARRAY['sandbox'::text, 'production'::text]))
);
CREATE INDEX IF NOT EXISTS "idx_apns_subscriptions_user_id" ON "apns_subscriptions" ("user_id");
