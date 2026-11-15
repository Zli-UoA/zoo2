#!/usr/bin/env bash
set -euo pipefail

: "${NOTES_TEST_DATABASE_URL:?Set NOTES_TEST_DATABASE_URL to a disposable PostgreSQL database with the main schema and notes migration applied}"
command -v psql >/dev/null

suffix="$(date +%s)_$$"
user_id="notes_race_user_${suffix}"
channel_id="notes_race_channel_${suffix}"
writer_app="notes_race_writer_${suffix}"
converter_app="notes_race_converter_${suffix}"
note_id="notes_race_note_${suffix}"
message_id="notes_race_message_${suffix}"
tmp_dir="$(mktemp -d)"

cleanup() {
  psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "DELETE FROM channels WHERE id = '$channel_id'; DELETE FROM users WHERE id = '$user_id';" >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "INSERT INTO users (id, user_name, display_name) VALUES ('$user_id', '$user_id', 'Race Test');
   INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ('$channel_id', '$channel_id', '$user_id', true);"

wait_for_activity() {
  local app="$1" predicate="$2"
  for _ in $(seq 1 100); do
    if psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c \
      "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE application_name = '$app' AND $predicate)" | rg -q '^t$'; then
      return 0
    fi
    sleep 0.1
  done
  echo "timed out waiting for PostgreSQL activity: $app ($predicate)" >&2
  return 1
}

expect_check_violation() {
  local output="$1" status="$2"
  if [[ "$status" -eq 0 ]] || ! rg -q '23514' "$output"; then
    cat "$output" >&2
    echo "expected SQLSTATE 23514 from the concurrent plaintext write" >&2
    return 1
  fi
}

# A note writer that gets FOR SHARE first must commit before conversion checks
# notes. The conversion then sees the committed note and rejects the switch.
PGAPPNAME="$writer_app" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -c "BEGIN; INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('$note_id', '$channel_id', 'race', 'plaintext', '$user_id'); SELECT pg_sleep(3); COMMIT" \
  >"$tmp_dir/writer-first.out" 2>&1 &
writer_pid=$!
wait_for_activity "$writer_app" "wait_event = 'PgSleep'"

set +e
PGAPPNAME="$converter_app" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "BEGIN; SELECT 1 FROM channels WHERE id = '$channel_id' FOR UPDATE; UPDATE channels SET encrypted = true WHERE id = '$channel_id'; COMMIT" \
  >"$tmp_dir/converter-first.out" 2>&1 &
converter_pid=$!
wait_for_activity "$converter_app" "wait_event_type = 'Lock'"
wait "$writer_pid"
wait "$converter_pid"
converter_status=$?
set -e
expect_check_violation "$tmp_dir/converter-first.out" "$converter_status"

# When conversion owns FOR UPDATE first, later note and message writers wait,
# then read encrypted=true and fail with the database guard's SQLSTATE.
psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "DELETE FROM channel_notes WHERE id = '$note_id';"
PGAPPNAME="$converter_app" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -c "BEGIN; SELECT 1 FROM channels WHERE id = '$channel_id' FOR UPDATE; UPDATE channels SET encrypted = true WHERE id = '$channel_id'; SELECT pg_sleep(3); COMMIT" \
  >"$tmp_dir/converter-locks.out" 2>&1 &
converter_pid=$!
wait_for_activity "$converter_app" "wait_event = 'PgSleep'"

set +e
PGAPPNAME="${writer_app}_note" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('${note_id}_after', '$channel_id', 'race', 'plaintext', '$user_id')" \
  >"$tmp_dir/note-after.out" 2>&1 &
note_pid=$!
PGAPPNAME="${writer_app}_message" psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "INSERT INTO messages (id, user_id, channel_id, content) VALUES ('$message_id', '$user_id', '$channel_id', 'plaintext')" \
  >"$tmp_dir/message-after.out" 2>&1 &
message_pid=$!
wait_for_activity "${writer_app}_note" "wait_event_type = 'Lock'"
wait_for_activity "${writer_app}_message" "wait_event_type = 'Lock'"
wait "$converter_pid"
if wait "$note_pid"; then note_status=0; else note_status=$?; fi
if wait "$message_pid"; then message_status=0; else message_status=$?; fi
set -e

expect_check_violation "$tmp_dir/note-after.out" "$note_status"
expect_check_violation "$tmp_dir/message-after.out" "$message_status"
echo "concurrent note/message writes and channel encryption were serialized; rejected writes returned SQLSTATE 23514"
