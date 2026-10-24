CREATE TABLE push_subscriptions (
  id text NOT NULL PRIMARY KEY,
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  endpoint text NOT NULL UNIQUE,
  created_at timestamp(3) NOT NULL
);
CREATE INDEX idx_push_subscriptions_user_id ON push_subscriptions (user_id);
