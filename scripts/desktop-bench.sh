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
#   window_ms   起動から、root ウィンドウの絵が何もない状態から最初に変わるまで
#   first_paint_ms 起動から、絵が空でなくなる (輝度の標準偏差が閾値以上になる) 最初の絵が出るまで
#   settled_ms  起動から、絵が最後に落ち着いた絵と同じになり、以後変わらなくなるまで
#               (入力欄のキャレットの点滅などで落ち着かないときは "-")
#   rss_mib     起動 8 秒後の、プロセスと子プロセス全体の RSS の合計 (共有ページは重複して数える)
#   rss_main_mib そのうち、起動したプロセス本体の RSS
#   pss_mib     全体の Pss の合計 (共有ページを按分する。smaps_rollup が読めるとき)
#   cpu_idle_pct 起動 8 秒後から 10 秒間の、プロセス全体の CPU 使用率 (1 コア = 100%)
#
# 画面は、飾りなし・最大化 (desktop-screenshot.sh と同じ openbox の設定) で撮るので、root ウィンドウ全体が
# クライアント領域になる。WM のタイトルバーや枠は含まない。
# 時間は、`xwd` で root を撮って比べた値。撮るのにも時間がかかるので (実際の間隔は poll_ms に出す)、
# 差が間隔以下のものは差として扱わない。目安であり、フレーム単位の計測ではない。
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
cat >"$tmp/rc.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<openbox_config xmlns="http://openbox.org/3.4/rc">
  <applications>
    <application class="*">
      <decor>no</decor>
      <maximized>yes</maximized>
    </application>
  </applications>
</openbox_config>
XML
openbox --config-file "$tmp/rc.xml" >"$tmp/openbox.log" 2>&1 &
pids+=($!)
sleep 1

export VK_ICD_FILENAMES=$ZOO_LVP_ICD
export ZOO_API_BASE_URL=http://127.0.0.1:9
export ZOO_CHECK_UPDATES=false
# WebKitGTK は Xvfb では GPU 合成が使えない。
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }
# root ウィンドウの輝度の標準偏差 (0 に近ければ空)
blank_score() { magick xwd:"$1" -colorspace Gray -format '%[fx:standard_deviation*1000]' info: 2>/dev/null || echo 0; }

# 子孫のプロセスを列挙する。
descendants() {
  local p=$1 c
  echo "$p"
  for c in $(pgrep -P "$p" 2>/dev/null || true); do descendants "$c"; done
}
pss_kib() { # pid... の Pss の合計 (KiB)。読めなければ 0
  local total=0 p v
  for p in "$@"; do
    v=$(awk '/^Pss:/ {print $2}' "/proc/$p/smaps_rollup" 2>/dev/null || echo 0)
    total=$(( total + ${v:-0} ))
  done
  echo "$total"
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
  frames=$tmp/frames$i
  mkdir -p "$frames"
  xwd -root -silent >"$frames/empty.xwd"
  empty_sum=$(md5sum <"$frames/empty.xwd" | cut -d' ' -f1)
  start=$(now_ms)
  "$bin" "${args[@]}" >"$tmp/app$i.log" 2>&1 &
  app=$!
  pids+=($app)
  # 起動から 8 秒、root の絵を撮り続け、変わったときだけ保存する。
  : >"$frames/log"
  polls=0
  prev=$empty_sum
  while :; do
    t=$(( $(now_ms) - start ))
    [ "$t" -lt 8000 ] || break
    kill -0 "$app" 2>/dev/null || { echo "起動できませんでした:" >&2; tail -5 "$tmp/app$i.log" >&2; exit 1; }
    xwd -root -silent >"$frames/cur.xwd"
    polls=$(( polls + 1 ))
    sum=$(md5sum <"$frames/cur.xwd" | cut -d' ' -f1)
    if [ "$sum" != "$prev" ]; then
      cp "$frames/cur.xwd" "$frames/f$polls.xwd"
      echo "$t $sum f$polls.xwd" >>"$frames/log"
      prev=$sum
    fi
    last_t=$t
    last_sum=$sum
  done
  poll_ms=$(( last_t / polls ))
  window_ms=$(head -1 "$frames/log" | cut -d' ' -f1)
  [ -n "$window_ms" ] || { echo "ウィンドウの絵が出ませんでした (8 秒)" >&2; exit 1; }
  # 最初に空でなくなった絵
  paint=
  while read -r t sum file; do
    s=$(blank_score "$frames/$file")
    if [ "${s%.*}" -ge 5 ] 2>/dev/null; then paint=$t; break; fi
  done <"$frames/log"
  [ -n "$paint" ] || paint=-
  # 最後に落ち着いた絵と同じになり、以後変わらなくなった時刻 (最後の変化の時刻)
  settled=-
  last_line=$(tail -1 "$frames/log")
  if [ "$(echo "$last_line" | cut -d' ' -f2)" = "$last_sum" ]; then settled=$(echo "$last_line" | cut -d' ' -f1); fi
  mapfile -t tree < <(descendants "$app")
  rss=$(rss_kib "${tree[@]}")
  rss_main=$(rss_kib "$app")
  pss=$(pss_kib "${tree[@]}")
  t0=$(cpu_ticks "${tree[@]}")
  sleep 10
  mapfile -t tree < <(descendants "$app")
  t1=$(cpu_ticks "${tree[@]}")
  hz=$(getconf CLK_TCK)
  cpu=$(awk -v a="$t0" -v b="$t1" -v hz="$hz" 'BEGIN { printf "%.1f", (b - a) / hz / 10 * 100 }')
  printf '%s run=%d window_ms=%s first_paint_ms=%s settled_ms=%s poll_ms=%d rss_mib=%.0f rss_main_mib=%.0f pss_mib=%.0f cpu_idle_pct=%s procs=%d\n' \
    "$name" "$i" "$window_ms" "$paint" "$settled" "$poll_ms" \
    "$(awk -v k="$rss" 'BEGIN{print k/1024}')" "$(awk -v k="$rss_main" 'BEGIN{print k/1024}')" "$(awk -v k="$pss" 'BEGIN{print k/1024}')" \
    "$cpu" "${#tree[@]}"
  kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
  rm -rf "$frames"
done
