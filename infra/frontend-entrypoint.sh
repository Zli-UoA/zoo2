#!/bin/sh
set -eu

# ビルド時に index.html から生成したインライン script の SHA-256 を、
# nginx 公式 entrypoint の envsubst（/etc/nginx/templates）へ渡す。
# ハッシュはビルド成果物と一致している必要があるため、実行時にファイルから読む。
CSP_SCRIPT_SRC="$(cat /etc/nginx/csp-script-src.txt)"
export CSP_SCRIPT_SRC

exec /docker-entrypoint.sh "$@"
