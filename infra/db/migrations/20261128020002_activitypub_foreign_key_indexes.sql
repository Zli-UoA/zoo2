-- atlas:txmode none

-- Create index "apub_deliveries_actor" to table: "apub_deliveries"
CREATE INDEX CONCURRENTLY IF NOT EXISTS "apub_deliveries_actor" ON "apub_deliveries" ("actor_id");
-- Create index "apub_follows_actor" to table: "apub_follows"
CREATE INDEX CONCURRENTLY IF NOT EXISTS "apub_follows_actor" ON "apub_follows" ("actor_id");
-- Create index "apub_remote_posts_actor" to table: "apub_remote_posts"
CREATE INDEX CONCURRENTLY IF NOT EXISTS "apub_remote_posts_actor" ON "apub_remote_posts" ("actor_id");
-- Create index "apub_remote_recipients_owner" to table: "apub_remote_recipients"
CREATE INDEX CONCURRENTLY IF NOT EXISTS "apub_remote_recipients_owner" ON "apub_remote_recipients" ("user_id", "post_id");
