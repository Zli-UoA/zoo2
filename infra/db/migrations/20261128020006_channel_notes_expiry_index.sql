-- atlas:txmode none

-- Older branch histories used this name for (deleted_at, id). IF NOT EXISTS
-- would silently preserve that incompatible definition. This new migration is
-- deliberately after both histories, so replace the index without rewriting
-- either already-applied migration or its Atlas checksum. Concurrent DDL keeps
-- note reads and writes available. Atlas records statement-level progress for
-- txmode none, so if CREATE INDEX CONCURRENTLY fails and leaves an invalid
-- index, drop it with DROP INDEX CONCURRENTLY IF EXISTS before retrying Atlas;
-- Atlas resumes at the failed CREATE statement instead of rerunning this DROP.
DROP INDEX CONCURRENTLY IF EXISTS "idx_channel_notes_expired";
CREATE INDEX CONCURRENTLY "idx_channel_notes_expired" ON "channel_notes" ("channel_id", "deleted_at", "id") WHERE (deleted_at IS NOT NULL);
