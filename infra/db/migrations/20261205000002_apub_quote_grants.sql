-- Create "apub_quote_grants" table
CREATE TABLE "apub_quote_grants" (
  "post_id" text NOT NULL,
  "user_id" text NOT NULL,
  "note_updated_at" timestamptz NOT NULL,
  PRIMARY KEY ("post_id", "user_id"),
  CONSTRAINT "apub_quote_grants_post_id_fkey" FOREIGN KEY ("post_id") REFERENCES "apub_remote_posts" ("id") ON UPDATE NO ACTION ON DELETE CASCADE,
  CONSTRAINT "apub_quote_grants_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
-- Create index "apub_quote_grants_user" to table: "apub_quote_grants"
CREATE INDEX "apub_quote_grants_user" ON "apub_quote_grants" ("user_id");
