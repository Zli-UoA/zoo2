#!/usr/bin/env bash
set -euo pipefail

: "${NOTES_TEST_DATABASE_URL:?Set NOTES_TEST_DATABASE_URL to a disposable PostgreSQL database with the main schema and notes migration applied}"
command -v psql >/dev/null
# wait_for_activity と期待値の検査に rg を使うため、fixture を作る前に要件を確認する。
command -v rg >/dev/null

suffix="$(date +%s)_$$"
user_id="notes_race_user_${suffix}"
channel_id="notes_race_channel_${suffix}"
writer_app="notes_race_writer_${suffix}"
converter_app="notes_race_converter_${suffix}"
note_id="notes_race_note_${suffix}"
message_id="notes_race_message_${suffix}"
tmp_dir="$(mktemp -d)"
writer_fifo="$tmp_dir/writer.fifo"
converter_fifo="$tmp_dir/converter.fifo"
mkfifo "$writer_fifo" "$converter_fifo"
writer_pid=""
converter_pid=""
note_pid=""
message_pid=""
writer_fd_open=false
converter_fd_open=false

cleanup() {
  if [[ "$writer_fd_open" == true ]]; then exec 3>&-; fi
  if [[ "$converter_fd_open" == true ]]; then exec 4>&-; fi
  for pid in "$writer_pid" "$converter_pid" "$note_pid" "$message_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "DELETE FROM channels WHERE id = '$channel_id'; DELETE FROM users WHERE id = '$user_id';" >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "INSERT INTO users (id, user_name, display_name) VALUES ('$user_id', '$user_id', 'Race Test');
   INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ('$channel_id', '$channel_id', '$user_id', true);"

bounded_psql() {
  # 接続と応答待ち全体に上限を設ける。観測用 psql が止まると待機ループが次の反復へ
  # 進まず、cleanup にも到達しなくなる。期限切れなら psql を終了させて失敗扱いにする。
  PGCONNECT_TIMEOUT=2 PGOPTIONS='-c statement_timeout=2000' \
    psql "$NOTES_TEST_DATABASE_URL" -X -Atq -v ON_ERROR_STOP=1 -c "$1" &
  local probe_pid=$!
  local waited=0
  while kill -0 "$probe_pid" 2>/dev/null; do
    if (( waited >= 30 )); then
      kill "$probe_pid" 2>/dev/null || true
      kill -KILL "$probe_pid" 2>/dev/null || true
      wait "$probe_pid" 2>/dev/null || true
      echo "observation query timed out: $1" >&2
      return 1
    fi
    sleep 0.1
    waited=$((waited + 1))
  done
  wait "$probe_pid"
}

wait_for_activity() {
  local app="$1" predicate="$2"
  for _ in $(seq 1 100); do
    if bounded_psql \
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
# Hold the writer's transaction open through a FIFO and release it only after
# the converter is confirmed to be waiting on the channel row lock.
# Each session names itself with SET application_name inside the session:
# libpq ignores PGAPPNAME when the connection URL already carries
# application_name, so the environment variable alone cannot label sessions.
psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -f - <"$writer_fifo" \
  >"$tmp_dir/writer-first.out" 2>&1 &
writer_pid=$!
exec 3>"$writer_fifo"
writer_fd_open=true
printf "SET application_name = '%s';\nBEGIN;\nINSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('%s', '%s', 'race', 'plaintext', '%s');\n" \
  "$writer_app" "$note_id" "$channel_id" "$user_id" >&3
wait_for_activity "$writer_app" \
  "state = 'idle in transaction' AND EXISTS (SELECT 1 FROM pg_locks l WHERE l.pid = pg_stat_activity.pid AND l.relation = 'channel_notes'::regclass AND l.granted)"

psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "SET application_name = '$converter_app'; BEGIN; SELECT 1 FROM channels WHERE id = '$channel_id' FOR UPDATE; UPDATE channels SET encrypted = true WHERE id = '$channel_id'; COMMIT" \
  >"$tmp_dir/converter-first.out" 2>&1 &
converter_pid=$!
wait_for_activity "$converter_app" "wait_event_type = 'Lock'"
printf "COMMIT;\n" >&3
exec 3>&-
writer_fd_open=false
wait "$writer_pid"
writer_pid=""
if wait "$converter_pid"; then converter_status=0; else converter_status=$?; fi
converter_pid=""
expect_check_violation "$tmp_dir/converter-first.out" "$converter_status"

# When conversion owns FOR UPDATE first, later note and message writers wait,
# then read encrypted=true and fail with the database guard's SQLSTATE. Hold the
# converter's transaction open through a FIFO until both writers are blocked.
psql "$NOTES_TEST_DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c \
  "DELETE FROM channel_notes WHERE id = '$note_id';"
psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -f - <"$converter_fifo" \
  >"$tmp_dir/converter-locks.out" 2>&1 &
converter_pid=$!
exec 4>"$converter_fifo"
converter_fd_open=true
printf "SET application_name = '%s';\nBEGIN;\nSELECT 1 FROM channels WHERE id = '%s' FOR UPDATE;\nUPDATE channels SET encrypted = true WHERE id = '%s';\n" \
  "$converter_app" "$channel_id" "$channel_id" >&4
wait_for_activity "$converter_app" \
  "state = 'idle in transaction' AND EXISTS (SELECT 1 FROM pg_locks l WHERE l.pid = pg_stat_activity.pid AND l.relation = 'channels'::regclass AND l.granted)"

psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "SET application_name = '${writer_app}_note'; INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ('${note_id}_after', '$channel_id', 'race', 'plaintext', '$user_id')" \
  >"$tmp_dir/note-after.out" 2>&1 &
note_pid=$!
psql "$NOTES_TEST_DATABASE_URL" -X -v ON_ERROR_STOP=1 -v VERBOSITY=verbose \
  -c "SET application_name = '${writer_app}_message'; INSERT INTO messages (id, user_id, channel_id, content) VALUES ('$message_id', '$user_id', '$channel_id', 'plaintext')" \
  >"$tmp_dir/message-after.out" 2>&1 &
message_pid=$!
wait_for_activity "${writer_app}_note" "wait_event_type = 'Lock'"
wait_for_activity "${writer_app}_message" "wait_event_type = 'Lock'"
printf "COMMIT;\n" >&4
exec 4>&-
converter_fd_open=false
wait "$converter_pid"
converter_pid=""
if wait "$note_pid"; then note_status=0; else note_status=$?; fi
note_pid=""
if wait "$message_pid"; then message_status=0; else message_status=$?; fi
message_pid=""

expect_check_violation "$tmp_dir/note-after.out" "$note_status"
expect_check_violation "$tmp_dir/message-after.out" "$message_status"
echo "concurrent note/message writes and channel encryption were serialized; rejected writes returned SQLSTATE 23514"
