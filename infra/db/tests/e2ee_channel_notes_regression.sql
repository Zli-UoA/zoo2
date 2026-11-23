\set ON_ERROR_STOP on

-- Run against a fresh database loaded from the main schema at a pinned commit.
-- FETCH_HEAD moves with every fetch and can already contain this branch's note
-- guards, which would make the migration below fail before either mode runs.
-- 7873efb7 is main after #595: it has the existing conversion guard and none
-- of this branch's 20261130030001..000009 migrations.
--   git show 7873efb7:infra/db/schema.sql | psql "$DATABASE_URL" -v ON_ERROR_STOP=1
--   psql "$DATABASE_URL" -v branch=on -v mode=branch -f infra/db/tests/e2ee_channel_notes_regression.sql
--   psql "$DATABASE_URL" -v branch=off -v mode=main -f infra/db/tests/e2ee_channel_notes_regression.sql
-- Branch mode removes main-only grants and restores the branch's immutable
-- encryption function. Main mode retains bot, webhook, and conversion guards.
-- The whole test rolls back, including the migration, so the DB can be reused.
BEGIN;
SET search_path = public;

\if :branch
DROP TABLE e2ee_bot_grants;
ALTER TABLE channel_user DISABLE TRIGGER channel_user_e2ee_reject_bots;
CREATE OR REPLACE FUNCTION e2ee_protect_channel_encryption() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'channels.encrypted cannot be changed' USING ERRCODE = '23514';
END $$;
\endif
SET test.mode = :'mode';
SELECT set_config('test.encryption_function_before', pg_get_functiondef('e2ee_protect_channel_encryption()'::regprocedure), true);

\ir ../migrations/20261130030003_e2ee_channel_notes.sql

DO $$
DECLARE
  state text;
BEGIN
  IF pg_get_functiondef('e2ee_protect_channel_encryption()'::regprocedure)
       <> current_setting('test.encryption_function_before') THEN
    RAISE EXCEPTION 'channel encryption protection function was replaced';
  END IF;
  INSERT INTO users (id, user_name, display_name, is_bot)
    VALUES ('notes-regression-user', 'notes-regression-user', 'Regression User', false),
           ('notes-regression-bot', 'notes-regression-bot', 'Regression Bot', true);
  INSERT INTO channels (id, channel_name, created_user_id, is_private, encrypted)
    VALUES ('notes-regression-branch', 'notes-regression-branch', 'notes-regression-user', true, false),
           ('notes-regression-bot-channel', 'notes-regression-bot-channel', 'notes-regression-user', true, false),
           ('notes-regression-webhook-channel', 'notes-regression-webhook-channel', 'notes-regression-user', true, false),
           ('notes-regression-granted-channel', 'notes-regression-granted-channel', 'notes-regression-user', true, false),
           ('notes-regression-note-channel', 'notes-regression-note-channel', 'notes-regression-user', true, false);

  IF current_setting('test.mode') = 'branch' THEN
    INSERT INTO channel_user (user_id, channel_id, joined_at)
      VALUES ('notes-regression-bot', 'notes-regression-branch', now());
    BEGIN
      UPDATE channels SET encrypted = true WHERE id = 'notes-regression-branch';
      RAISE EXCEPTION 'branch unexpectedly allowed channel encryption';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;
  ELSE
    INSERT INTO channel_user (user_id, channel_id, joined_at)
      VALUES ('notes-regression-bot', 'notes-regression-bot-channel', now());
    BEGIN
      UPDATE channels SET encrypted = true WHERE id = 'notes-regression-bot-channel';
      RAISE EXCEPTION 'ungranted bot unexpectedly allowed channel encryption';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;

    INSERT INTO channel_webhooks (id, channel_id, name, secret, user_id)
      VALUES ('notes-regression-webhook', 'notes-regression-webhook-channel', 'Webhook', 'secret', 'notes-regression-user');
    BEGIN
      UPDATE channels SET encrypted = true WHERE id = 'notes-regression-webhook-channel';
      RAISE EXCEPTION 'channel with a webhook unexpectedly encrypted';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;

    INSERT INTO e2ee_bot_grants (channel_id, bot_user_id)
      VALUES ('notes-regression-granted-channel', 'notes-regression-bot');
    INSERT INTO channel_user (user_id, channel_id, joined_at)
      VALUES ('notes-regression-bot', 'notes-regression-granted-channel', now());
    UPDATE channels SET encrypted = true WHERE id = 'notes-regression-granted-channel';

    INSERT INTO channel_notes (id, channel_id, title, content, created_user_id)
      VALUES ('notes-regression-note', 'notes-regression-note-channel', 'Plaintext', 'stored note', 'notes-regression-user');
    BEGIN
      UPDATE channels SET encrypted = true WHERE id = 'notes-regression-note-channel';
      RAISE EXCEPTION 'channel with plaintext notes unexpectedly encrypted';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;

    -- A tombstone inside the 24-hour restore window still blocks encryption,
    -- on its own channel; one past the window does not.
    INSERT INTO channels (id, channel_name, created_user_id, is_private, encrypted)
      VALUES ('notes-regression-recent-deleted-channel', 'notes-regression-recent-deleted-channel', 'notes-regression-user', true, false),
             ('notes-regression-stale-deleted-channel', 'notes-regression-stale-deleted-channel', 'notes-regression-user', true, false);
    INSERT INTO channel_notes (id, channel_id, title, content, created_user_id)
      VALUES ('notes-regression-recent-deleted-note', 'notes-regression-recent-deleted-channel', 'Plaintext', 'deleted 23h ago', 'notes-regression-user'),
             ('notes-regression-stale-deleted-note', 'notes-regression-stale-deleted-channel', 'Plaintext', 'deleted 25h ago', 'notes-regression-user');
    UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '23 hours'
      WHERE id = 'notes-regression-recent-deleted-note';
    UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours'
      WHERE id = 'notes-regression-stale-deleted-note';
    BEGIN
      UPDATE channels SET encrypted = true WHERE id = 'notes-regression-recent-deleted-channel';
      RAISE EXCEPTION 'channel with a restorable deleted note unexpectedly encrypted';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;
    UPDATE channels SET encrypted = true WHERE id = 'notes-regression-stale-deleted-channel';

    UPDATE channels SET encrypted = true WHERE id = 'notes-regression-branch';
    BEGIN
      INSERT INTO channel_notes (id, channel_id, title, content, created_user_id)
        VALUES ('notes-regression-rejected-note', 'notes-regression-branch', 'Plaintext', 'must reject', 'notes-regression-user');
      RAISE EXCEPTION 'plaintext note unexpectedly inserted into encrypted channel';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;
    BEGIN
      INSERT INTO messages (id, user_id, channel_id, content)
        VALUES ('notes-regression-plaintext-message', 'notes-regression-user', 'notes-regression-branch', 'must reject');
      RAISE EXCEPTION 'plaintext message unexpectedly inserted into encrypted channel';
    EXCEPTION WHEN check_violation THEN
      GET STACKED DIAGNOSTICS state = RETURNED_SQLSTATE;
      IF state <> '23514' THEN RAISE; END IF;
    END;
  END IF;
END $$;

ROLLBACK;
