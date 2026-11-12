#!/usr/bin/env bash
# デスクトップクライアントの起動時間・メモリ・CPU を Xvfb 上で測る。Linux 用。`nix develop` の中で実行する。
#
# 使い方: scripts/desktop-bench.sh [-n 回数] 名前 実行ファイル [引数...]
# 例:
#   scripts/desktop-bench.sh gpui desktop/target/release/zoo2-desktop run
#   ZOO_DESKTOP_UI=leptos scripts/desktop-bench.sh tauri-leptos desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
#   ZOO_DESKTOP_UI=remote scripts/desktop-bench.sh tauri-remote desktop-tauri/src-tauri/target/release/zoo2-desktop-tauri
#
# 測るもの (ログイン前の画面で測る。実際の AuthZ ログインと backend は使わない):
#   first_paint_ms  起動から、root ウィンドウの絵が空でなくなるまで (100 ms 刻みで撮って比べる)
#   rss_mib         描画 5 秒後の、プロセスと子プロセス全体の RSS の合計 (共有ページは重複して数える)
#   rss_main_mib    そのうち、起動したプロセス本体の RSS
#   cpu_idle_pct    描画 5 秒後から 10 秒間の、プロセス全体の CPU 使用率 (1 コア = 100%)
# スクロール時の CPU は、ログインと、メッセージの入った backend が要るのでここでは測らない。
# 描画は lavapipe (CPU の Vulkan) なので、実機の GPU の値とは違う。
set -euo pipefail

runs=3
while getopts "n:" opt; do
  case "$opt" in
    n) runs=$OPTARG ;;
    *) exit 2 ;;
  esac
done
shift $((OPTIND - 1))
[ $# -ge 2 ] || { echo "使い方: $0 [-n 回数] 名前 実行ファイル [引数...]" >&2; exit 2; }
name=$1
bin=$(realpath "$2")
shift 2
args=("$@")

for cmd in Xvfb openbox xwd magick; do
  command -v "$cmd" >/dev/null || { echo "$cmd が見つかりません。nix develop の中で実行してください" >&2; exit 1; }
done
: "${ZOO_LVP_ICD:?ZOO_LVP_ICD が未設定です。nix develop の中で実行してください}"

tmp=$(mktemp -d)
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
  rm -rf "$tmp"
}
trap cleanup EXIT

Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$tmp/display" >"$tmp/xvfb.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 100); do [ -s "$tmp/display" ] && break; sleep 0.1; done
export DISPLAY=":$(tr -d '\n' <"$tmp/display")"
unset WAYLAND_DISPLAY
openbox >"$tmp/openbox.log" 2>&1 &
pids+=($!)
sleep 1

export VK_ICD_FILENAMES=$ZOO_LVP_ICD
export ZOO_API_BASE_URL=http://127.0.0.1:9
export ZOO_CHECK_UPDATES=false
# WebKitGTK は Xvfb では GPU 合成が使えない。
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }
# root ウィンドウの輝度の標準偏差 (0 に近ければ空)
blank_score() { xwd -root -silent | magick xwd:- -colorspace Gray -format '%[fx:standard_deviation*1000]' info: 2>/dev/null || echo 0; }

# 子孫のプロセスを列挙する。
descendants() {
  local p=$1 c
  echo "$p"
  for c in $(pgrep -P "$p" 2>/dev/null || true); do descendants "$c"; done
}
rss_kib() { # pid... の VmRSS の合計 (KiB)
  local total=0 p v
  for p in "$@"; do
    v=$(awk '/^VmRSS:/ {print $2}' "/proc/$p/status" 2>/dev/null || echo 0)
    total=$(( total + ${v:-0} ))
  done
  echo "$total"
}
cpu_ticks() { # pid... の utime+stime の合計 (clock ticks)
  local total=0 p t
  for p in "$@"; do
    t=$(awk '{print $14 + $15}' "/proc/$p/stat" 2>/dev/null || echo 0)
    total=$(( total + ${t:-0} ))
  done
  echo "$total"
}

for i in $(seq 1 "$runs"); do
  HOME=$tmp/home$i XDG_CONFIG_HOME=$tmp/config$i XDG_RUNTIME_DIR=$tmp/run$i
  export HOME XDG_CONFIG_HOME XDG_RUNTIME_DIR
  mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"
  start=$(now_ms)
  "$bin" "${args[@]}" >"$tmp/app$i.log" 2>&1 &
  app=$!
  pids+=($app)
  paint=
  for _ in $(seq 1 600); do
    kill -0 "$app" 2>/dev/null || { echo "起動できませんでした:" >&2; tail -5 "$tmp/app$i.log" >&2; exit 1; }
    s=$(blank_score)
    if [ "${s%.*}" -ge 5 ] 2>/dev/null; then paint=$(( $(now_ms) - start )); break; fi
    sleep 0.05
  done
  [ -n "$paint" ] || { echo "描画を確認できませんでした (30 秒)" >&2; exit 1; }
  sleep 5
  mapfile -t tree < <(descendants "$app")
  rss=$(rss_kib "${tree[@]}")
  rss_main=$(rss_kib "$app")
  t0=$(cpu_ticks "${tree[@]}")
  sleep 10
  mapfile -t tree < <(descendants "$app")
  t1=$(cpu_ticks "${tree[@]}")
  hz=$(getconf CLK_TCK)
  cpu=$(awk -v a="$t0" -v b="$t1" -v hz="$hz" 'BEGIN { printf "%.1f", (b - a) / hz / 10 * 100 }')
  printf '%s run=%d first_paint_ms=%d rss_mib=%.0f rss_main_mib=%.0f cpu_idle_pct=%s procs=%d\n' \
    "$name" "$i" "$paint" "$(awk -v k="$rss" 'BEGIN{print k/1024}')" "$(awk -v k="$rss_main" 'BEGIN{print k/1024}')" "$cpu" "${#tree[@]}"
  kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
done
