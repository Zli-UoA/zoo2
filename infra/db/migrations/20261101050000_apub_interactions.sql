ALTER TABLE apub_follows ADD COLUMN activity_json text;
CREATE TABLE apub_reactions (
  post_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text REFERENCES users(id) ON DELETE CASCADE,
  activity_id text NOT NULL UNIQUE,
  activity_json text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(post_id, actor_id)
);
CREATE TABLE apub_undone_activities (
  activity_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(activity_id, actor_id, user_id)
);
CREATE TABLE apub_notifications (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL,
  activity_id text NOT NULL,
  kind text NOT NULL CHECK(kind IN ('follow_request','follow_accepted','like')),
  post_id text,
  is_read boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(user_id,activity_id,kind)
);
CREATE INDEX apub_notifications_owner ON apub_notifications(user_id,created_at DESC);
CREATE TRIGGER apub_reactions_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_notifications_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_notifications FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
