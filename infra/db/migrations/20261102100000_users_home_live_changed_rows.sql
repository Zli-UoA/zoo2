-- Avoid invalidating every client for periodic zero-row/no-op user updates.
DROP TRIGGER users_home_live ON users;
CREATE TRIGGER users_home_live AFTER INSERT OR DELETE ON users FOR EACH ROW EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER users_update_home_live AFTER UPDATE ON users FOR EACH ROW WHEN (OLD.* IS DISTINCT FROM NEW.*) EXECUTE FUNCTION notify_home_timeline();
