-- Modify "messages" table
ALTER TABLE "messages" ADD COLUMN "parent_id" text NULL, ADD CONSTRAINT "messages_parent_id_fkey" FOREIGN KEY ("parent_id") REFERENCES "messages" ("id") ON UPDATE CASCADE ON DELETE RESTRICT;

-- Create index "idx_messages_parent_id" to table: "messages"
CREATE INDEX "idx_messages_parent_id" ON "messages" ("parent_id");
