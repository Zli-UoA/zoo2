#!/bin/sh
set -eu

# ビルド時に index.html から生成したインライン script の SHA-256 を、
# nginx 公式 entrypoint の envsubst（/etc/nginx/templates）へ渡す。
# ハッシュはビルド成果物と一致している必要があるため、実行時にファイルから読む。
CSP_SCRIPT_SRC="$(cat /etc/nginx/csp-script-src.txt)"
export CSP_SCRIPT_SRC

# Restrict characters before interpolating the public endpoint into nginx syntax.
MEDIA_PROXY_URL="${MEDIA_PROXY_URL:-}"
case "$MEDIA_PROXY_URL" in
  *[!a-zA-Z0-9:/._~%-]*) echo "Invalid MEDIA_PROXY_URL" >&2; exit 1 ;;
esac
CSP_MEDIA_PROXY_SRC=""
case "$MEDIA_PROXY_URL" in
  https://*) CSP_MEDIA_PROXY_SRC="$(printf '%s' "$MEDIA_PROXY_URL" | sed 's#^\(https://[^/]*\).*#\1#')" ;;
  http://localhost:*|http://127.0.0.1:*) CSP_MEDIA_PROXY_SRC="$(printf '%s' "$MEDIA_PROXY_URL" | sed 's#^\(http://[^/]*\).*#\1#')" ;;
  //*) echo "MEDIA_PROXY_URL must not be protocol-relative" >&2; exit 1 ;;
  /*|"") ;;
  *) echo "MEDIA_PROXY_URL must be HTTPS or a same-origin path" >&2; exit 1 ;;
esac
export MEDIA_PROXY_URL CSP_MEDIA_PROXY_SRC

exec /docker-entrypoint.sh "$@"
