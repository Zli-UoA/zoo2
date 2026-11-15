#!/usr/bin/env bash
# Staged rollout; each batch commits independently. DATABASE_URL is never printed.
set -euo pipefail
: "${DATABASE_URL:?Set DATABASE_URL for the intended database}"
export PGCONNECT_TIMEOUT=10
export PGOPTIONS="${PGOPTIONS:-} -c lock_timeout=2s -c statement_timeout=30s"
run() { psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 "$@"; }
case "${1:-status}" in
  status)
    run -c "SELECT phase FROM thread_rollout; SELECT count(*) AS remaining_threads FROM messages m WHERE parent_id IS NULL AND (thread_activity_at IS NOT NULL OR EXISTS(SELECT 1 FROM messages child WHERE child.parent_id=m.id)) AND NOT EXISTS(SELECT 1 FROM thread_migration_completed d WHERE d.thread_id=m.id); SELECT count(*) AS nested_messages FROM messages m JOIN messages p ON p.id=m.parent_id WHERE p.parent_id IS NOT NULL; SELECT count(*) AS nested_home_posts FROM home_posts h WHERE h.parent_id IS NOT NULL AND h.parent_id IS DISTINCT FROM home_thread_root(h.parent_id);"
    ;;
  start)
    # Preflight raises on cycles; refuse missing ancestors before enabling normalization.
    run -1 -c "DO \$\$ BEGIN IF EXISTS(SELECT 1 FROM messages WHERE thread_root(id) IS NULL) THEN RAISE EXCEPTION 'Repair missing thread ancestors first'; END IF; IF EXISTS(SELECT 1 FROM messages child JOIN messages parent ON parent.id=child.parent_id WHERE child.channel_id<>parent.channel_id) THEN RAISE EXCEPTION 'Repair cross-channel parents first'; END IF; PERFORM home_thread_root(id) FROM home_posts; END \$\$; UPDATE thread_rollout SET phase='normalizing' WHERE phase='collecting';"
    ;;
  batch)
    count="${2:-1}"
    [[ "$count" =~ ^[0-9]+$ ]] || { echo 'Batch size must be an integer' >&2; exit 1; }
    run -c "SELECT migrate_thread_batch($count);"
    run -c "SELECT migrate_home_thread_batch(100);"
    ;;
  finish)
    run -1 -c "DO \$\$ BEGIN IF (SELECT phase FROM thread_rollout)<>'normalizing' OR EXISTS(SELECT 1 FROM messages m WHERE parent_id IS NULL AND (thread_activity_at IS NOT NULL OR EXISTS(SELECT 1 FROM messages child WHERE child.parent_id=m.id)) AND NOT EXISTS(SELECT 1 FROM thread_migration_completed d WHERE d.thread_id=m.id)) OR EXISTS(SELECT 1 FROM messages m JOIN messages p ON p.id=m.parent_id WHERE p.parent_id IS NOT NULL) OR EXISTS(SELECT 1 FROM home_posts h WHERE h.parent_id IS NOT NULL AND h.parent_id IS DISTINCT FROM home_thread_root(h.parent_id)) THEN RAISE EXCEPTION 'Thread migration is incomplete'; END IF; END \$\$; UPDATE thread_rollout SET phase='ready';"
    ;;
  *) echo 'Usage: migrate-threads.sh {status|start|batch [size]|finish}' >&2; exit 1 ;;
esac
