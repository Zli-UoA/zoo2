-- 解除後に遅れて届く登録要求を識別するため、解除記録を一定期間保持する。
CREATE TABLE "apns_revocations" (
  "id" text NOT NULL,
  "revoked_at" timestamp(3) NOT NULL,
  PRIMARY KEY ("id")
);
CREATE INDEX "idx_apns_revocations_revoked_at" ON "apns_revocations" ("revoked_at");

CREATE TABLE "apns_subscriptions" (
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
CREATE INDEX "idx_apns_subscriptions_user_id" ON "apns_subscriptions" ("user_id");
