-- Create "phone_sessions" table
CREATE TABLE "phone_sessions" (
  "id" text NOT NULL,
  "user_id" text NOT NULL,
  "line_id" text NOT NULL,
  "line_revision" timestamptz NOT NULL,
  "peer_id" text NOT NULL,
  "gateway_session_id" bigint NULL,
  "gateway_handle_id" bigint NULL,
  "lease_owner" text NULL,
  "expires_at" timestamptz NOT NULL DEFAULT (now() + '00:01:00'::interval),
  "created_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("id"),
  CONSTRAINT "phone_sessions_line_id_fkey" FOREIGN KEY ("line_id") REFERENCES "sip_lines" ("id") ON UPDATE NO ACTION ON DELETE CASCADE,
  CONSTRAINT "phone_sessions_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
-- Create index "phone_sessions_line_idx" to table: "phone_sessions"
CREATE INDEX "phone_sessions_line_idx" ON "phone_sessions" ("line_id");
-- Create index "phone_sessions_user_expiry" to table: "phone_sessions"
CREATE INDEX "phone_sessions_user_expiry" ON "phone_sessions" ("user_id", "expires_at");
