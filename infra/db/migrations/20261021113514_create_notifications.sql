-- Create "notifications" table
CREATE TABLE "notifications" ("id" text NOT NULL, "user_id" text NOT NULL, "actor_id" text NOT NULL, "channel_id" text NOT NULL, "message_id" text NOT NULL, "notification_type" text NOT NULL, "is_read" boolean NOT NULL DEFAULT false, "created_at" timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY ("id"), CONSTRAINT "notifications_actor_id_fkey" FOREIGN KEY ("actor_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE CASCADE, CONSTRAINT "notifications_channel_id_fkey" FOREIGN KEY ("channel_id") REFERENCES "channels" ("id") ON UPDATE CASCADE ON DELETE CASCADE, CONSTRAINT "notifications_message_id_fkey" FOREIGN KEY ("message_id") REFERENCES "messages" ("id") ON UPDATE CASCADE ON DELETE CASCADE, CONSTRAINT "notifications_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE CASCADE);
-- Create index "idx_notifications_channel_id" to table: "notifications"
CREATE INDEX "idx_notifications_channel_id" ON "notifications" ("channel_id");
-- Create index "idx_notifications_message_id" to table: "notifications"
CREATE INDEX "idx_notifications_message_id" ON "notifications" ("message_id");
-- Create index "idx_notifications_user_id" to table: "notifications"
CREATE INDEX "idx_notifications_user_id" ON "notifications" ("user_id");
-- Create index "idx_notifications_user_id_is_read" to table: "notifications"
CREATE INDEX "idx_notifications_user_id_is_read" ON "notifications" ("user_id", "is_read");

-- Set REPLICA IDENTITY FULL for ElectricSQL sync
ALTER TABLE "notifications" REPLICA IDENTITY FULL;

