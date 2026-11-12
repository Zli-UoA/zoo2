#!/usr/bin/env bash
# GPUI 版デスクトップの常駐を Xvfb 上で確かめる。Linux (X11) 用。`nix develop` の中で実行する。
#
# 使い方: scripts/desktop-resident-smoke.sh 実行ファイル
#
# 確かめること:
#   1. 起動してウィンドウが出る
#   2. 2 つ目の起動が、すぐ終了コード 0 で終わり、最初のインスタンスは生きている
#   3. 閉じる操作 (WM_DELETE_WINDOW) で、終了せずに最小化される (WM_STATE が Iconic)
#   4. 2 つ目の起動の「前に出す」で、最小化から戻る (WM_STATE が Normal)
#   5. 最初のインスタンスを止めて残ったソケットでも、次の起動が通る (古いソケットの作り直し)
# 設定とソケットは一時ディレクトリに置き、実際の設定やトークンには触れない。
set -euo pipefail

bin=${1:?使い方: $0 実行ファイル}
bin=$(realpath "$bin")
for cmd in Xvfb openbox xprop python3 pkg-config; do
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
fail() { echo "NG: $*" >&2; exit 1; }
ok() { echo "OK: $*"; }

Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$tmp/display" >"$tmp/xvfb.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 100); do [ -s "$tmp/display" ] && break; sleep 0.1; done
export DISPLAY=":$(tr -d '\n' <"$tmp/display")"
unset WAYLAND_DISPLAY
openbox >"$tmp/openbox.log" 2>&1 &
pids+=($!)
sleep 1

mkdir -p "$tmp/run" "$tmp/config" "$tmp/home"
chmod 700 "$tmp/run"
export XDG_RUNTIME_DIR=$tmp/run XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data HOME=$tmp/home
export VK_ICD_FILENAMES=$ZOO_LVP_ICD
export ZOO_API_BASE_URL=http://127.0.0.1:9
export ZOO_CHECK_UPDATES=false

# 閉じる操作を送る小さな補助。libX11 を ctypes で呼ぶ。
cat >"$tmp/xclose.py" <<'PY'
import ctypes, ctypes.util, subprocess, sys
libdir = subprocess.check_output(["pkg-config", "--variable=libdir", "x11"]).decode().strip()
x = ctypes.CDLL(libdir + "/libX11.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XInternAtom.restype = ctypes.c_ulong
x.XDefaultRootWindow.restype = ctypes.c_ulong
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
x.XFlush.argtypes = [ctypes.c_void_p]
d = x.XOpenDisplay(None)
win = int(sys.argv[1], 0)
class ClientMessage(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]
class XEvent(ctypes.Union):
    _fields_ = [("xclient", ClientMessage), ("pad", ctypes.c_long * 24)]
ev = XEvent()
ev.xclient.type = 33  # ClientMessage
ev.xclient.window = win
ev.xclient.message_type = x.XInternAtom(d, b"WM_PROTOCOLS", 0)
ev.xclient.format = 32
ev.xclient.data[0] = x.XInternAtom(d, b"WM_DELETE_WINDOW", 0)
ev.xclient.data[1] = 0
x.XSendEvent(d, win, 0, 0, ctypes.byref(ev))
x.XFlush(d)
PY

client_window() {
  xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -o '0x[0-9a-f]*' | head -1
}
wm_state() {
  xprop -id "$1" WM_STATE 2>/dev/null | grep -o 'Iconic\|Normal' | head -1 || true
}
wait_for() { # 説明 条件コマンド
  for _ in $(seq 1 100); do
    if eval "$2" >/dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
  fail "$1 (時間切れ)"
}

"$bin" run >"$tmp/app1.log" 2>&1 &
app=$!
pids+=($app)
wait_for "ウィンドウが出る" '[ -n "$(client_window)" ]'
win=$(client_window)
ok "1. ウィンドウが出た ($win)"

start=$(date +%s%N)
"$bin" run >"$tmp/app2.log" 2>&1 && code=0 || code=$?
elapsed=$(( ($(date +%s%N) - start) / 1000000 ))
[ "$code" = 0 ] || fail "2つ目の起動が終了コード $code"
kill -0 "$app" 2>/dev/null || fail "2つ目の起動で最初のインスタンスが落ちた"
ok "2. 2つ目の起動は ${elapsed} ms で終了コード 0、最初のインスタンスは生きている"

python3 "$tmp/xclose.py" "$win"
wait_for "閉じる操作で最小化される" '[ "$(wm_state '"$win"')" = Iconic ]'
kill -0 "$app" 2>/dev/null || fail "閉じる操作でプロセスが終了した"
ok "3. 閉じる操作で終了せず最小化された (WM_STATE=Iconic)"

"$bin" run >"$tmp/app3.log" 2>&1
wait_for "2つ目の起動で最小化から戻る" '[ "$(wm_state '"$win"')" = Normal ]'
ok "4. 2つ目の起動で最小化から戻った (WM_STATE=Normal)"

kill -9 "$app"
wait "$app" 2>/dev/null || true
[ -e "$XDG_RUNTIME_DIR/zoo2-desktop.sock" ] || fail "kill -9 後にソケットが残っていない (前提が崩れた)"
"$bin" run >"$tmp/app4.log" 2>&1 &
app=$!
pids+=($app)
sleep 3
kill -0 "$app" 2>/dev/null || fail "古いソケットが残っていると起動できない"
ok "5. 古いソケットが残っていても、作り直して起動した"
echo "すべて通りました"
