#!/usr/bin/env bash
# PostgreSQLで省略された参照先PKなどを解決してからLiamへ渡す。
set -euo pipefail
cd "$(dirname "$0")"
name="zoo2-liam-erd-$$"
work="$(mktemp -d)"
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

docker run -d --rm --name "$name" \
  -e POSTGRES_PASSWORD=erd -e POSTGRES_DB=erd postgres:16 >/dev/null
ready=false
for _ in $(seq 1 60); do
  if docker exec "$name" pg_isready -h 127.0.0.1 -U postgres -d erd -q; then
    ready=true
    break
  fi
  sleep 1
done
if [ "$ready" != true ]; then
  echo 'ER図生成用PostgreSQLが起動しませんでした' >&2
  exit 1
fi
docker exec -i "$name" psql -U postgres -d erd -v ON_ERROR_STOP=1 -q \
  < ../../infra/db/schema.sql >/dev/null
docker exec "$name" pg_dump -U postgres -d erd --schema-only --no-owner --no-privileges \
  > "$work/schema.sql"
# PostgreSQL 16の新しいpg_dumpが付加するpsql専用コマンドはSQLパーサーへ渡さない。
sed '/^\\/d' "$work/schema.sql" > "$work/input.sql"
liam erd build --input "$work/input.sql" --format postgres --output-dir dist
