-- Create "sip_lines" table
CREATE TABLE "sip_lines" (
  "id" text NOT NULL,
  "name" text NOT NULL,
  "owner_user_id" text NULL,
  "websocket_path" text NOT NULL,
  "sip_domain" text NOT NULL,
  "username" text NOT NULL,
  "password_ciphertext" text NOT NULL,
  "phone_number" text NOT NULL DEFAULT '',
  "created_by" text NOT NULL,
  "updated_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("id"),
  CONSTRAINT "sip_lines_created_by_fkey" FOREIGN KEY ("created_by") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE NO ACTION,
  CONSTRAINT "sip_lines_owner_user_id_fkey" FOREIGN KEY ("owner_user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE NO ACTION
);
-- Create index "sip_lines_owner_idx" to table: "sip_lines"
CREATE INDEX "sip_lines_owner_idx" ON "sip_lines" ("owner_user_id");
-- Create "sip_line_members" table
CREATE TABLE "sip_line_members" (
  "line_id" text NOT NULL,
  "user_id" text NOT NULL,
  PRIMARY KEY ("line_id", "user_id"),
  CONSTRAINT "sip_line_members_line_id_fkey" FOREIGN KEY ("line_id") REFERENCES "sip_lines" ("id") ON UPDATE NO ACTION ON DELETE CASCADE,
  CONSTRAINT "sip_line_members_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE NO ACTION
);
-- Create index "sip_line_members_user_idx" to table: "sip_line_members"
CREATE INDEX "sip_line_members_user_idx" ON "sip_line_members" ("user_id");
-- Create "sip_phones" table
CREATE TABLE "sip_phones" (
  "id" text NOT NULL,
  "line_id" text NOT NULL,
  "name" text NOT NULL,
  "extension" text NOT NULL,
  PRIMARY KEY ("id"),
  CONSTRAINT "sip_phones_line_id_fkey" FOREIGN KEY ("line_id") REFERENCES "sip_lines" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
