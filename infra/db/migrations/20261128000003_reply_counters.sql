-- atlas:txmode none

-- Recovery marker for the unmerged preview revision which failed before its
-- first statement. Atlas needs this filename to resume that partial revision.
-- The actual, idempotent implementation follows main's latest migration in
-- 20261128020001_home_timeline_seek.sql and 20261128020003_reply_counters.sql.
SELECT 1;
