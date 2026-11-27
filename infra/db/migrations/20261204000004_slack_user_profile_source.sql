-- atlas:txmode none
-- Preserve where imported Slack metadata came from so a later ZIP profile can
-- replace fallback metadata without confusing a real linked Zoo account.
ALTER TABLE slack_users
  ADD COLUMN profile_source text NOT NULL DEFAULT 'unknown';
ALTER TABLE slack_users
  ADD CONSTRAINT slack_users_profile_source_check
    CHECK (profile_source IN ('unknown', 'fallback', 'profile')) NOT VALID;
ALTER TABLE slack_users
  VALIDATE CONSTRAINT slack_users_profile_source_check;
