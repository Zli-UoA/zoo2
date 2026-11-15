#!/usr/bin/env bash
set -euo pipefail

: "${NOTES_TEST_DATABASE_URL:?Set NOTES_TEST_DATABASE_URL to a disposable PostgreSQL database with canonical migrations through 20261128000002 applied}"
command -v psql >/dev/null
command -v atlas >/dev/null

migration_dir="$(cd "$(dirname "$0")/.." && pwd)/migrations"
atlas_dir="file://${migration_dir}"
target_version=20261128000003
suffix="$(date +%s)_$$"
user_id="notes_migration_user_${suffix}"
channel_id="notes_migration_channel_${suffix}"
note_id="notes_migration_note_${suffix}"
writer_app="notes_migration_writer_${suffix}"
migration_app="notes_migration_apply_${suffix}"
tmp_dir="$(mktemp -d)"

cleanup() {
  psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "DELETE FROM channels WHERE id = '$channel_id'; DELETE FROM users WHERE id = '$user_id';" >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "INSERT INTO users (id, user_name, display_name) VALUES ('$user_id', '$user_id', 'Migration lock test');
   INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ('$channel_id', '$channel_id', '$user_id', true);"

# Simulate a pre-migration notes writer holding ROW EXCLUSIVE on channel_notes.
# The migration must not wait while retaining its earlier channels lock.
PGAPPNAME="$writer_app" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -c "BEGIN; INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('$note_id', '$channel_id', 'plain', 'note', '$user_id'); SELECT pg_sleep(5); COMMIT" \
  >"$tmp_dir/writer.out" 2>&1 &
writer_pid=$!

writer_sleeping=false
for _ in $(seq 1 100); do
  if psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE application_name = '$writer_app' AND wait_event = 'PgSleep')" | rg -q '^t$'; then
    writer_sleeping=true
    break
  fi
  sleep 0.1
done
if [[ "$writer_sleeping" != true ]]; then
  cat "$tmp_dir/writer.out" >&2
  echo "timed out waiting for the legacy notes writer to hold its transaction" >&2
  exit 1
fi

set +e
PGAPPNAME="$migration_app" atlas migrate apply --dir "$atlas_dir" --url "$NOTES_TEST_DATABASE_URL" \
  --to-version "$target_version" >"$tmp_dir/migration.out" 2>&1 &
migration_pid=$!
# Waiting for the migration before the writer proves it does not remain stuck:
# the legacy writer has a bounded sleep and will release its lock either way.
wait "$migration_pid"
migration_status=$?
wait "$writer_pid"
writer_status=$?
set -e

if [[ "$writer_status" -ne 0 ]]; then
  cat "$tmp_dir/writer.out" >&2
  exit "$writer_status"
fi
if [[ "$migration_status" -eq 0 ]] || ! rg -q '55P03' "$tmp_dir/migration.out"; then
  cat "$tmp_dir/migration.out" >&2
  echo "expected migration to fail immediately with SQLSTATE 55P03 while notes are locked" >&2
  exit 1
fi

if psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname IN ('channels_encrypted_reject_notes', 'channel_notes_e2ee_reject_plaintext'))" | rg -q '^t$'; then
  echo "failed migration left a partial trigger behind" >&2
  exit 1
fi
if psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT EXISTS (SELECT 1 FROM atlas_schema_revisions.atlas_schema_revisions WHERE version = '$target_version')" | rg -q '^t$'; then
  echo "failed migration was recorded as applied" >&2
  exit 1
fi

# The failed deploy leaves this migration pending. A later deployment reruns
# Atlas after the writer commits and installs both guards.
PGAPPNAME="${migration_app}_retry" atlas migrate apply --dir "$atlas_dir" --url "$NOTES_TEST_DATABASE_URL" \
  --to-version "$target_version" >"$tmp_dir/retry.out" 2>&1
psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT count(*) = 2 FROM pg_trigger WHERE tgname IN ('channels_encrypted_reject_notes', 'channel_notes_e2ee_reject_plaintext')" | rg -q '^t$'
echo "Atlas deploy failed fast with 55P03 without recording the migration, then applied it successfully on retry"
