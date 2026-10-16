-- Create "message_reactions" table
CREATE TABLE "message_reactions" (
  "id" text NOT NULL,
  "message_id" text NOT NULL,
  "user_id" text NOT NULL,
  "emoji" text NOT NULL,
  "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY ("id"),
  CONSTRAINT "message_reactions_unique" UNIQUE ("message_id", "user_id", "emoji"),
  CONSTRAINT "message_reactions_message_id_fkey" FOREIGN KEY ("message_id") REFERENCES "messages" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT "message_reactions_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE RESTRICT
);

-- Create index "idx_message_reactions_message_id" to table: "message_reactions"
CREATE INDEX "idx_message_reactions_message_id" ON "message_reactions" ("message_id");

-- Create index "idx_message_reactions_user_id" to table: "message_reactions"
CREATE INDEX "idx_message_reactions_user_id" ON "message_reactions" ("user_id");

-- Create index "idx_messages_channel_id" to table: "messages"
CREATE INDEX "idx_messages_channel_id" ON "messages" ("channel_id");

-- Add unique constraint on role.role_name
ALTER TABLE "role" ADD CONSTRAINT "role_role_name_key" UNIQUE ("role_name");

-- Set REPLICA IDENTITY FULL for ElectricSQL sync
ALTER TABLE "message_reactions" REPLICA IDENTITY FULL;

-- Seed admin role master
INSERT INTO "role" ("id", "role_name") VALUES ('role_admin', 'admin') ON CONFLICT DO NOTHING;
