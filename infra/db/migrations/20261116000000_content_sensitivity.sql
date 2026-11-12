-- Label channels and images without changing existing records during rollout.
ALTER TABLE channels ADD COLUMN is_nsfw boolean NOT NULL DEFAULT false;
ALTER TABLE attachments ADD COLUMN is_sensitive boolean NOT NULL DEFAULT false;
