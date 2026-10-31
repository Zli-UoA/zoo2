#!/bin/sh
set -eu

# ビルド時に index.html から生成したインライン script の SHA-256 を、
# nginx 公式 entrypoint の envsubst（/etc/nginx/templates）へ渡す。
# ハッシュはビルド成果物と一致している必要があるため、実行時にファイルから読む。
CSP_SCRIPT_SRC="$(cat /etc/nginx/csp-script-src.txt)"
export CSP_SCRIPT_SRC
# React-only images retain their original connect-src allowlist.
CSP_COMPOSE_CONNECT_SRC=""
if [ "${ZOO_COMPOSE_ENABLED:-false}" = "true" ]; then
  CSP_COMPOSE_CONNECT_SRC=" https://www.gstatic.com ${CSP_APP_IMAGE_SRC:-}"
fi
export CSP_COMPOSE_CONNECT_SRC

# Public endpoints are interpolated into nginx syntax; allow URL path characters only.
endpoint_origin() {
  case "$1" in
    *[!a-zA-Z0-9:/._~%-]*) echo "Invalid public preview endpoint" >&2; return 1 ;;
  esac
  case "$1" in
    https://*) printf '%s' "$1" | sed 's#^\(https://[^/]*\).*#\1#' ;;
    http://localhost:*|http://127.0.0.1:*) printf '%s' "$1" | sed 's#^\(http://[^/]*\).*#\1#' ;;
    //*) echo "Preview endpoint must not be protocol-relative" >&2; return 1 ;;
    /*|"") ;;
    *) echo "Preview endpoint must be HTTPS or a same-origin path" >&2; return 1 ;;
  esac
}
MEDIA_PROXY_URL="${MEDIA_PROXY_URL:-}"
SUMMALY_URL="${SUMMALY_URL:-}"
CSP_MEDIA_PROXY_SRC="$(endpoint_origin "$MEDIA_PROXY_URL")"
CSP_SUMMALY_SRC="$(endpoint_origin "$SUMMALY_URL")"
export MEDIA_PROXY_URL SUMMALY_URL CSP_MEDIA_PROXY_SRC CSP_SUMMALY_SRC

exec /docker-entrypoint.sh "$@"
