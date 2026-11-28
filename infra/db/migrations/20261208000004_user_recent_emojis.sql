-- Create "user_recent_emojis" table
CREATE TABLE "user_recent_emojis" (
  "user_id" text NOT NULL,
  "emojis" text[] NOT NULL,
  PRIMARY KEY ("user_id"),
  CONSTRAINT "user_recent_emojis_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE,
  CONSTRAINT "user_recent_emojis_emojis_check" CHECK (cardinality(emojis) <= 16)
);
