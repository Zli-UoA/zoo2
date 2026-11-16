#!/bin/sh
set -eu

# Only this unmerged PR's preview revision failed before its first statement.
# Complete its empty revision before switching to the renumbered migrations.
# Keeping the obsolete file in the canonical directory would make Atlas reject
# upgrades from newer main histories as non-linear.
if [ "${1:-}" = migrate ] && [ "${2:-}" = apply ] && [ -n "${DATABASE_URL:-}" ]; then
  revisions=$(psql -XqAt "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "SELECT to_regclass('atlas_schema_revisions.atlas_schema_revisions') IS NOT NULL")
  if [ "$revisions" = t ]; then
    total=$(psql -XqAt "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "SELECT total FROM atlas_schema_revisions.atlas_schema_revisions WHERE version='20261128000003' AND applied=0 AND total BETWEEN 1 AND 100")
    if [ -n "$total" ]; then
      recovery=$(mktemp -d)
      trap 'rm -rf "$recovery"' EXIT HUP INT TERM
      # Keep all no-op statements and revision updates in one transaction so
      # an interrupted recovery remains at applied=0 and can be retried.
      printf '%s\n' '-- atlas:txmode file' '' > "$recovery/20261128000003_reply_counters.sql"
      i=0
      while [ "$i" -lt "$total" ]; do
        printf '%s\n' 'SELECT 1;' >> "$recovery/20261128000003_reply_counters.sql"
        i=$((i + 1))
      done
      atlas migrate hash --dir "file://$recovery"
      atlas migrate apply --dir "file://$recovery" --url "$DATABASE_URL"
      rm -rf "$recovery"
      trap - EXIT HUP INT TERM
    fi
  fi
fi
exec atlas "$@"
