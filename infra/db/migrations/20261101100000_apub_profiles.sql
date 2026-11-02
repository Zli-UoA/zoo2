CREATE TABLE apub_profiles (
  user_id text PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  summary text NOT NULL DEFAULT '' CHECK (char_length(summary)<=1000),
  display_name text CHECK (display_name IS NULL OR (char_length(display_name) BETWEEN 1 AND 100))
);
ALTER TABLE apub_actors ADD COLUMN avatar jsonb, ADD COLUMN summary text NOT NULL DEFAULT '';
CREATE TRIGGER apub_profiles_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_profiles FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
