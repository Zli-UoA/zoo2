CREATE TABLE apns_subscriptions (
  id text NOT NULL PRIMARY KEY,
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  device_token text NOT NULL,
  environment text NOT NULL CHECK (environment IN ('sandbox', 'production')),
  revocation_hash text NOT NULL,
  created_at timestamp(3) NOT NULL,
  UNIQUE (device_token, environment)
);
CREATE INDEX idx_apns_subscriptions_user_id ON apns_subscriptions (user_id);

-- Retain revocation tombstones so a delayed registration cannot resurrect a logged-out device.
CREATE TABLE apns_revocations (
    id text NOT NULL PRIMARY KEY,
    revoked_at timestamp(3) NOT NULL
);
