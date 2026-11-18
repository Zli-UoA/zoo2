#!/usr/bin/env bash
set -euo pipefail

# 署名処理の出力を判定用に一時保存するため、他のユーザーから読めないようにする。
build_log=$(umask 077; mktemp "${TMPDIR:-/tmp}/zoo2-tauri-build.XXXXXX")
trap 'rm -f "$build_log"' EXIT

for attempt in 1 2 3; do
  if cargo tauri build --target universal-apple-darwin --bundles app,dmg --config "$1" -- --locked 2>&1 | tee "$build_log"; then
    exit 0
  else
    statuses=("${PIPESTATUS[@]}")
  fi

  # ログの欠落や中断は再試行で隠さず、その終了コードを返す。
  if [ "${statuses[1]}" -ne 0 ]; then
    exit "${statuses[1]}"
  fi
  if [ "${statuses[0]}" -ge 128 ] || [ "$attempt" -eq 3 ]; then
    exit "${statuses[0]}"
  fi

  # 証明書・公証などの恒久的な失敗まで繰り返さないよう、実際の codesign エラーに限定する。
  if ! grep -Eq ': A timestamp was expected but was not found\.$' "$build_log" ||
     ! grep -Fq 'failed to run command codesign: failed to sign app' "$build_log"; then
    exit "${statuses[0]}"
  fi

  delay=$((attempt * 30))
  echo "::warning::Apple の署名タイムスタンプを取得できませんでした。${delay} 秒後に macOS のビルド・署名・公証を再試行します（次は $((attempt + 1))/3 回目）。"
  sleep "$delay"
done
