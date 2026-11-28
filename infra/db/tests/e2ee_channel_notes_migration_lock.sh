#!/usr/bin/env bash
set -euo pipefail

: "${NOTES_TEST_DATABASE_URL:?Set NOTES_TEST_DATABASE_URL to a disposable PostgreSQL database with canonical migrations through 20261204000002 applied}"
command -v psql >/dev/null
command -v atlas >/dev/null

migration_dir="$(cd "$(dirname "$0")/.." && pwd)/migrations"
atlas_dir="file://${migration_dir}"
target_version=20261204000003
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
watchdog_pid=""

cleanup() {
  # Stop Atlas before releasing the writer's lock: an orphaned Atlas could
  # otherwise apply the migration after this script has already failed.
  if [[ -n "${watchdog_pid:-}" ]] && kill -0 "$watchdog_pid" 2>/dev/null; then
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
  fi
  if [[ -n "${migration_pid:-}" ]] && kill -0 "$migration_pid" 2>/dev/null; then
    kill "$migration_pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "$migration_pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$migration_pid" 2>/dev/null || true
    wait "$migration_pid" 2>/dev/null || true
  fi
  exec 3>&- 2>/dev/null || true
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

printf "SET application_name = '%s';\nBEGIN;\nINSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('%s', '%s', 'plain', 'note', '%s');\n" \
  "$writer_app" "$note_id" "$channel_id" "$user_id" >&3

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

# Run Atlas directly in the background (not through a helper subshell), so
# migration_pid is Atlas itself. Killing a subshell would leave its Atlas child
# running, and that orphan could apply the migration after this test failed.
PGAPPNAME="$migration_app" atlas migrate apply --dir "$atlas_dir" --url "$NOTES_TEST_DATABASE_URL" \
  --to-version "$target_version" >"$tmp_dir/migration.out" 2>&1 &
migration_pid=$!

# Enforce an independent hang-detection timeout while the writer keeps holding
# its lock deterministically until Atlas exits. The watchdog terminates the DB
# session (which unblocks Atlas) and leaves a marker that distinguishes a
# timeout from the expected lock_timeout failure.
(
  for _ in $(seq 1 300); do
    if [[ -f "$tmp_dir/migration.done" ]]; then
      exit 0
    fi
    sleep 0.1
  done
  printf 'timeout\n' >"$tmp_dir/migration.timeout"
  # Bound the connect and query phases: when the database is unresponsive,
  # this psql must not keep the watchdog from terminating Atlas.
  PGCONNECT_TIMEOUT=2 PGOPTIONS='-c statement_timeout=2000' psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = '$migration_app';" >/dev/null 2>&1 || true
  # Terminate Atlas itself too, so the main flow's wait cannot hang forever on
  # a migration that is not waiting on a database lock.
  kill "$migration_pid" 2>/dev/null || true
  for _ in $(seq 1 50); do
    kill -0 "$migration_pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL "$migration_pid" 2>/dev/null || true
) &
watchdog_pid=$!

migration_status=0
wait "$migration_pid" || migration_status=$?
migration_pid=""
: >"$tmp_dir/migration.done"
if [[ -n "$watchdog_pid" ]]; then
  kill "$watchdog_pid" 2>/dev/null || true
  wait "$watchdog_pid" 2>/dev/null || true
  watchdog_pid=""
fi

printf "COMMIT;\n" >&3
exec 3>&-
if wait "$writer_pid"; then writer_status=0; else writer_status=$?; fi
writer_pid=""

if [[ "$writer_status" -ne 0 ]]; then
  cat "$tmp_dir/writer.out" >&2
  exit "$writer_status"
fi
# The watchdog marker means Atlas overran the budget even if it happened to
# fail with 55P03 right after the timeout. Check it before the SQLSTATE so an
# over-budget run can never pass through the rollback checks below.
if [[ -f "$tmp_dir/migration.timeout" ]]; then
  cat "$tmp_dir/migration.out" >&2
  echo "the migration lock watchdog fired before Atlas finished" >&2
  exit 1
fi
if [[ "$migration_status" -eq 0 ]] || ! rg -q '55P03' "$tmp_dir/migration.out"; then
  cat "$tmp_dir/migration.out" >&2
  if [[ -f "$tmp_dir/migration.timeout" ]] && [[ "$migration_status" -ne 0 ]]; then
    echo "timed out waiting for migration to fail fast with SQLSTATE 55P03" >&2
  else
    echo "expected migration to fail immediately with SQLSTATE 55P03 while notes are locked" >&2
  fi
  exit 1
fi

trigger_rows="$(psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname IN ('channels_encrypted_reject_notes', 'channel_notes_e2ee_reject_plaintext'))" \
  2>"$tmp_dir/trigger.err")" || {
  cat "$tmp_dir/trigger.err" >&2
  echo "failed to inspect triggers after the failed migration" >&2
  exit 1
}
if [[ "$trigger_rows" == t ]]; then
  echo "failed migration left a partial trigger behind" >&2
  exit 1
fi
version_rows="$(psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT EXISTS (SELECT 1 FROM atlas_schema_revisions.atlas_schema_revisions WHERE version = '$target_version')" \
  2>"$tmp_dir/version.err")" || {
  cat "$tmp_dir/version.err" >&2
  echo "failed to inspect the recorded migration version" >&2
  exit 1
}
if [[ "$version_rows" == t ]]; then
  echo "failed migration was recorded as applied" >&2
  exit 1
fi

# The failed deploy leaves this migration pending. A later deployment reruns
# Atlas after the writer commits and installs both guards. Surface the retry's
# output and exit code when it fails; with plain `set -e` the redirect would
# hide retry.out and lose the status.
retry_status=0
PGAPPNAME="${migration_app}_retry" atlas migrate apply --dir "$atlas_dir" --url "$NOTES_TEST_DATABASE_URL" \
  --to-version "$target_version" >"$tmp_dir/retry.out" 2>&1 || retry_status=$?
if [[ "$retry_status" -ne 0 ]]; then
  cat "$tmp_dir/retry.out" >&2
  echo "the retry migration apply failed with exit code $retry_status" >&2
  exit "$retry_status"
fi
psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
  "SELECT count(*) = 2 FROM pg_trigger WHERE tgname IN ('channels_encrypted_reject_notes', 'channel_notes_e2ee_reject_plaintext')" | rg -q '^t$'
echo "Atlas deploy failed fast with 55P03 without recording the migration, then applied it successfully on retry"
