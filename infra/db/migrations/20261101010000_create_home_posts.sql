-- Home posts are explicitly public and separate from private channel messages.
CREATE TABLE home_posts (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  content text NOT NULL CHECK (char_length(content) BETWEEN 1 AND 1000 AND btrim(content) <> ''),
  created_at timestamptz NOT NULL DEFAULT statement_timestamp()
);
CREATE INDEX idx_home_posts_created ON home_posts (created_at DESC, id DESC);
CREATE INDEX idx_home_posts_author ON home_posts (user_id, created_at DESC, id DESC);
