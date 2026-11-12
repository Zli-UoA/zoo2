#!/usr/bin/env bash
# デスクトップクライアントの画面を撮る。Linux 用。`nix develop` の中で実行する。
#
# 使い方:
#   scripts/desktop-screenshot.sh [-w 待ち秒数] [-s 幅x高さ] 出力.png -- 実行ファイル [引数...]
# 例:
#   scripts/desktop-screenshot.sh /tmp/gallery-dark.png -- desktop/target/debug/zoo2-desktop gallery --theme dark
#
# 仮想ディスプレイ (Xvfb) に openbox を立てて実行ファイルを動かし、root ウィンドウを撮る。
# WM が無いと GPUI のウィンドウは黒く描かれるので、openbox は必須。
# 描画は lavapipe (CPU の Vulkan)。ICD は devShell の ZOO_LVP_ICD を使う。
set -euo pipefail

wait_secs=6
geometry=1440x900
while getopts "w:s:" opt; do
  case "$opt" in
    w) wait_secs=$OPTARG ;;
    s) geometry=$OPTARG ;;
    *) exit 2 ;;
  esac
done
shift $((OPTIND - 1))

if [ $# -lt 3 ] || [ "$2" != "--" ]; then
  echo "使い方: $0 [-w 秒] [-s 幅x高さ] 出力.png -- 実行ファイル [引数...]" >&2
  exit 2
fi
out=$1
shift 2

for cmd in Xvfb openbox xwd magick; do
  command -v "$cmd" >/dev/null || { echo "$cmd が見つかりません。nix develop の中で実行してください" >&2; exit 1; }
done
: "${ZOO_LVP_ICD:?ZOO_LVP_ICD が未設定です。nix develop の中で実行してください}"

[ -f "$ZOO_LVP_ICD" ] || { echo "lavapipe の ICD が見つかりません: $ZOO_LVP_ICD (nix develop の mesa に含まれていない環境かもしれません)" >&2; exit 1; }

tmp=$(mktemp -d)
pids=()
cleanup() {
  # 自分で起動したプロセスだけを PID で止める。
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  for pid in "${pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  rm -rf "$tmp"
}
trap cleanup EXIT

# 空いているディスプレイ番号は Xvfb に選ばせ、fd 3 から受け取る。
Xvfb -displayfd 3 -screen 0 "${geometry}x24" -nolisten tcp 3>"$tmp/display" >"$tmp/xvfb.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 100); do
  [ -s "$tmp/display" ] && break
  sleep 0.1
done
[ -s "$tmp/display" ] || { echo "Xvfb が起動しませんでした" >&2; cat "$tmp/xvfb.log" >&2; exit 1; }
export DISPLAY=":$(tr -d '\n' <"$tmp/display")"

openbox >"$tmp/openbox.log" 2>&1 &
pids+=($!)
sleep 1

export VK_ICD_FILENAMES=$ZOO_LVP_ICD
"$@" >"$tmp/app.log" 2>&1 &
app=$!
pids+=("$app")

sleep "$wait_secs"
if ! kill -0 "$app" 2>/dev/null; then
  echo "実行ファイルが終了しました:" >&2
  cat "$tmp/app.log" >&2
  exit 1
fi

xwd -root -silent | magick xwd:- "$out"
echo "$out に保存しました (DISPLAY=$DISPLAY)"
# 実行ファイルの出力を見たいときは ZOO_SCREENSHOT_LOG=1 を付ける。
if [ -n "${ZOO_SCREENSHOT_LOG:-}" ]; then
  cat "$tmp/app.log"
fi
