#!/usr/bin/env bash
# infra/db/schema.sql から ER 図 (Mermaid) を生成し、docs/db-erd.md に書き出す。
#
#   infra/db/erd.sh          docs/db-erd.md を再生成する
#   infra/db/erd.sh --check  再生成した内容が docs/db-erd.md と一致するか検査する(書き込まない)
#
# 必要なもの: atlas (https://atlasgo.io) と docker。
#
# schema.sql にはトリガーと関数があり、Atlas の無償版は file:// の schema.sql をそのまま
# 読めない(Pro 機能)。そのため使い捨ての postgres:16 に schema.sql を流し込み、
# その DB を検査する。検査時に関数とトリガーは除外する(ER 図には不要)。
set -euo pipefail

cd "$(dirname "$0")"
out="../../docs/db-erd.md"
name="zoo-erd-$$"
pw="erd"

cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  [ -z "${tmp:-}" ] || rm -f "$tmp"
}
trap cleanup EXIT

start_db() {
  docker run -d --rm --name "$name" \
    -e POSTGRES_PASSWORD="$pw" -e POSTGRES_DB=erd \
    -p 127.0.0.1::5432 postgres:16 >/dev/null
  for _ in $(seq 1 60); do
    docker exec "$name" pg_isready -U postgres -d erd -q && return 0
    sleep 1
  done
  echo "postgres が起動しませんでした" >&2
  return 1
}

render() {
  start_db
  docker exec -i "$name" psql -U postgres -d erd -v ON_ERROR_STOP=1 -q <schema.sql >/dev/null
  port="$(docker port "$name" 5432/tcp | head -n1 | sed 's/.*://')"

  cat <<'HEADER'
# DB の ER 図

`infra/db/schema.sql` から自動生成しています。直接編集しないでください。

スキーマを変えたら `infra/db/erd.sh` を実行し、更新された `docs/db-erd.md` を commit してください。
CI(`ci-db-erd`)が `infra/db/erd.sh --check` で更新漏れを検査します。

HEADER
  echo '```mermaid'
  atlas schema inspect \
    --url "postgres://postgres:${pw}@localhost:${port}/erd?search_path=public&sslmode=disable" \
    --exclude '*[type=function]' \
    --exclude '*.*[type=trigger]' \
    --format '{{ mermaid . }}'
  echo '```'
}

case "${1:-}" in
  "")
    tmp="$(mktemp)"
    render >"$tmp"
    cat "$tmp" >"$out"
    echo "wrote docs/db-erd.md"
    ;;
  --check)
    tmp="$(mktemp)"
    render >"$tmp"
    if ! diff -u "$out" "$tmp"; then
      echo "docs/db-erd.md が古くなっています。infra/db/erd.sh を実行して commit してください。" >&2
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac
