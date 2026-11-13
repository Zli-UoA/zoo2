#!/usr/bin/env bash
# infra/db/schema.sql から ER 図 (Mermaid) を生成し、docs/db-erd.md と docs/db-erd/*.md に書き出す。
#
#   infra/db/erd.sh          ER 図を再生成する
#   infra/db/erd.sh --check  再生成した内容が docs/ の ER 図と一致するか検査する(書き込まない)
#
# 必要なもの: atlas (https://atlasgo.io) と docker。
#
# schema.sql にはトリガーと関数があり、Atlas の無償版は file:// の schema.sql をそのまま
# 読めない(Pro 機能)。そのため使い捨ての postgres:16 に schema.sql を流し込み、
# その DB を検査する。検査時に関数とトリガーは除外する(ER 図には不要)。
#
# 図が大きくなりすぎるので、テーブル名の接頭辞で機能ごとに分割する(group_of)。
# どの接頭辞にも当てはまらないテーブルは core に入るので、テーブルを足しても図から漏れない。
# 分割した図に別グループのテーブルへの外部キーがある場合、相手は属性なしの箱として描かれる。
set -euo pipefail

cd "$(dirname "$0")"
docs="../../docs"
name="zoo-erd-$$"
pw="erd"

# グループ名: 表示名 | 説明
groups=(
  "core|コア(ユーザー、チャンネル、メッセージ、通知など)"
  "apps|アプリ連携(apps と app_*)"
  "activitypub|ActivityPub(apub_*)"
  "e2ee|E2EE(e2ee_*)"
  "slack|Slack 取り込み(slack_*)"
)

work=""
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  [ -z "$work" ] || rm -rf "$work"
}
trap cleanup EXIT

start_db() {
  docker run -d --rm --name "$name" \
    -e POSTGRES_PASSWORD="$pw" -e POSTGRES_DB=erd \
    -p 127.0.0.1::5432 postgres:16 >/dev/null
  for _ in $(seq 1 60); do
    docker exec "$name" pg_isready -h 127.0.0.1 -U postgres -d erd -q && return 0
    sleep 1
  done
  echo "postgres が起動しませんでした" >&2
  return 1
}

# 全テーブルの Mermaid を標準出力に出す。
inspect_all() {
  start_db
  docker exec -i "$name" psql -U postgres -d erd -v ON_ERROR_STOP=1 -q <schema.sql >/dev/null
  local port
  port="$(docker port "$name" 5432/tcp | head -n1 | sed 's/.*://')"
  atlas schema inspect \
    --url "postgres://postgres:${pw}@localhost:${port}/erd?search_path=public&sslmode=disable" \
    --exclude '*[type=function]' \
    --exclude '*.*[type=trigger]' \
    --format '{{ mermaid . }}'
}

# 引数のグループに属するテーブルの定義と、そのテーブルが持つ外部キーだけを出す。
split_group() {
  awk -v g="$1" '
    function group_of(n) {
      if (n == "apps" || n ~ /^app_/) return "apps"
      if (n ~ /^apub_/) return "activitypub"
      if (n ~ /^e2ee_/) return "e2ee"
      if (n ~ /^slack_/) return "slack"
      return "core"
    }
    /^erDiagram/ { print; next }
    /^    [A-Za-z0-9_]+ \{$/ { cur = $1; inblock = 1; keep = (group_of(cur) == g) }
    inblock { if (keep) print; if ($0 ~ /^    \}$/) inblock = 0; next }
    /^    [A-Za-z0-9_]+ [|}o]/ { if (group_of($1) == g) print }
  '
}

table_count() {
  grep -cE '^    [A-Za-z0-9_]+ \{$' || true
}

# $1 の下に db-erd.md と db-erd/*.md を作る。
render() {
  local root="$1" all="$1/.all.mmd"
  mkdir -p "$root/db-erd"
  inspect_all >"$all"

  local index="$root/db-erd.md" entry key desc n
  cat >"$index" <<'HEADER'
# DB の ER 図

`infra/db/schema.sql` から自動生成しています。直接編集しないでください。

スキーマを変えたら `infra/db/erd.sh` を実行し、更新された `docs/db-erd.md` と `docs/db-erd/` を commit してください。
CI(`ci-db-erd`)が `infra/db/erd.sh --check` で更新漏れを検査します。

図が大きくなりすぎるので、テーブル名の接頭辞で機能ごとに分けています。
別の図のテーブルを参照する外部キーは、相手のテーブルが属性なしの箱で描かれます。

| 図 | 内容 | テーブル数 |
|---|---|---|
HEADER

  for entry in "${groups[@]}"; do
    key="${entry%%|*}"
    desc="${entry#*|}"
    split_group "$key" <"$all" >"$root/.part.mmd"
    n="$(table_count <"$root/.part.mmd")"
    echo "| [${key}](db-erd/${key}.md) | ${desc} | ${n} |" >>"$index"
    {
      echo "# DB の ER 図: ${key}"
      echo
      echo "${desc}。[一覧に戻る](../db-erd.md)"
      echo
      echo '`infra/db/schema.sql` から自動生成しています。直接編集しないでください。'
      echo
      echo '```mermaid'
      cat "$root/.part.mmd"
      echo '```'
    } >"$root/db-erd/${key}.md"
  done
  rm -f "$all" "$root/.part.mmd"
}

work="$(mktemp -d)"
render "$work"

case "${1:-}" in
  "")
    rm -rf "$docs/db-erd"
    cp -r "$work/db-erd" "$docs/db-erd"
    cp "$work/db-erd.md" "$docs/db-erd.md"
    echo "wrote docs/db-erd.md and docs/db-erd/"
    ;;
  --check)
    status=0
    diff -u "$docs/db-erd.md" "$work/db-erd.md" || status=1
    diff -ru "$docs/db-erd" "$work/db-erd" || status=1
    if [ "$status" -ne 0 ]; then
      echo "ER 図が古くなっています。infra/db/erd.sh を実行して commit してください。" >&2
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac
