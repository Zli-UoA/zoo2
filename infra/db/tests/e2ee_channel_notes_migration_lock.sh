#!/usr/bin/env bash
set -euo pipefail

: "${NOTES_TEST_DATABASE_URL:?Set NOTES_TEST_DATABASE_URL to a disposable PostgreSQL database with canonical migrations through 20261128030002 applied}"
command -v psql >/dev/null
command -v atlas >/dev/null

migration_dir="$(cd "$(dirname "$0")/.." && pwd)/migrations"
atlas_dir="file://${migration_dir}"
target_version=20261128030003
suffix="$(date +%s)_$$"
user_id="notes_migration_user_${suffix}"
channel_id="notes_migration_channel_${suffix}"
note_id="notes_migration_note_${suffix}"
writer_app="notes_migration_writer_${suffix}"
migration_app="notes_migration_apply_${suffix}"
tmp_dir="$(mktemp -d)"
writer_fifo="$tmp_dir/writer.fifo"
mkfifo "$writer_fifo"
writer_pid=""
migration_pid=""

cleanup() {
  exec 3>&- 2>/dev/null || true
  if [[ -n "${migration_pid:-}" ]] && kill -0 "$migration_pid" 2>/dev/null; then
    kill "$migration_pid" 2>/dev/null || true
    wait "$migration_pid" 2>/dev/null || true
  fi
  if [[ -n "${writer_pid:-}" ]] && kill -0 "$writer_pid" 2>/dev/null; then
    kill "$writer_pid" 2>/dev/null || true
    wait "$writer_pid" 2>/dev/null || true
  fi
  psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "DELETE FROM channels WHERE id = '$channel_id'; DELETE FROM users WHERE id = '$user_id';" >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "INSERT INTO users (id, user_name, display_name) VALUES ('$user_id', '$user_id', 'Migration lock test');
   INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ('$channel_id', '$channel_id', '$user_id', true);"

# Simulate a pre-migration notes writer holding ROW EXCLUSIVE on channel_notes.
# Synchronize explicitly via a FIFO instead of a fixed pg_sleep timeout so slow
# Atlas startup cannot race ahead of or outlast the lock window.
PGAPPNAME="$writer_app" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -f - <"$writer_fifo" >"$tmp_dir/writer.out" 2>&1 &
writer_pid=$!
exec 3>"$writer_fifo"

printf "BEGIN;\nINSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('%s', '%s', 'plain', 'note', '%s');\n" \
  "$note_id" "$channel_id" "$user_id" >&3

writer_holding_lock=false
for _ in $(seq 1 100); do
  if psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity a JOIN pg_locks l ON l.pid = a.pid WHERE a.application_name = '$writer_app' AND a.state = 'idle in transaction' AND l.relation = 'channel_notes'::regclass AND l.granted)" | rg -q '^t$'; then
    writer_holding_lock=true
    break
  fi
  sleep 0.1
done
if [[ "$writer_holding_lock" != true ]]; then
  cat "$tmp_dir/writer.out" >&2
  echo "timed out waiting for the legacy notes writer to hold its lock" >&2
  exit 1
fi

(
  status=0
  PGAPPNAME="$migration_app" atlas migrate apply --dir "$atlas_dir" --url "$NOTES_TEST_DATABASE_URL" \
    --to-version "$target_version" >"$tmp_dir/migration.out" 2>&1 || status=$?
  printf '%s\n' "$status" >"$tmp_dir/migration.status.tmp"
  mv "$tmp_dir/migration.status.tmp" "$tmp_dir/migration.status"
) &
migration_pid=$!

# Enforce an independent hang-detection timeout while the writer keeps holding
# its lock deterministically until Atlas exits.
migration_finished=false
for _ in $(seq 1 300); do
  if [[ -f "$tmp_dir/migration.status" ]]; then
    migration_finished=true
    break
  fi
  sleep 0.1
done
if [[ "$migration_finished" != true ]]; then
  psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = '$migration_app';" >/dev/null 2>&1 || true
  cat "$tmp_dir/migration.out" >&2 || true
  echo "timed out waiting for migration to fail fast with SQLSTATE 55P03" >&2
  exit 1
fi
wait "$migration_pid" 2>/dev/null || true
migration_pid=""
migration_status="$(<"$tmp_dir/migration.status")"

printf "COMMIT;\n" >&3
exec 3>&-
if wait "$writer_pid"; then writer_status=0; else writer_status=$?; fi
writer_pid=""

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
