ALTER TABLE apub_inbox_receipts DROP CONSTRAINT apub_inbox_receipts_pkey;
ALTER TABLE apub_inbox_receipts ADD PRIMARY KEY(id,actor_id,user_id);
CREATE TABLE apub_remote_tombstones (
  id text NOT NULL,
  actor_id text NOT NULL REFERENCES apub_actors(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(id,actor_id)
);
-- Preserve early Delete receipts without reserving another actor's canonical Note ID.
INSERT INTO apub_remote_tombstones(id,actor_id) SELECT id,actor_id FROM apub_remote_posts WHERE deleted_at IS NOT NULL;
DELETE FROM apub_remote_posts WHERE deleted_at IS NOT NULL;
ALTER TABLE apub_emoji_images ADD COLUMN last_seen_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX apub_emoji_images_last_seen ON apub_emoji_images(last_seen_at);
