-- Invalidation only: no post contents or identities are broadcast.
CREATE FUNCTION notify_home_timeline() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('home_timeline', 'refresh');
    RETURN NULL;
END;
$$;
CREATE TRIGGER home_posts_home_live AFTER INSERT OR UPDATE OR DELETE ON home_posts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_remote_posts_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_remote_posts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_remote_recipients_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_remote_recipients FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_follows_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_follows FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_actors_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_actors FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channels_home_live AFTER INSERT OR UPDATE OR DELETE ON channels FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channel_user_home_live AFTER INSERT OR UPDATE OR DELETE ON channel_user FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER users_home_live AFTER INSERT OR UPDATE OR DELETE ON users FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER user_role_home_live AFTER INSERT OR UPDATE OR DELETE ON user_role FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
