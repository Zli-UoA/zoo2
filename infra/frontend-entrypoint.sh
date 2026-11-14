#!/bin/sh
set -eu

# ビルド時に index.html から生成したインライン script の SHA-256 を、
# nginx 公式 entrypoint の envsubst（/etc/nginx/templates）へ渡す。
# ハッシュはビルド成果物と一致している必要があるため、実行時にファイルから読む。
CSP_SCRIPT_SRC="$(cat /etc/nginx/csp-script-src.txt)"
export CSP_SCRIPT_SRC
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
CSP_AVATAR_SRC="$(endpoint_origin "${CSP_AVATAR_SRC:-https://avater.zliusercontent.com}")"
export CSP_AVATAR_SRC
MEDIA_PROXY_URL="${MEDIA_PROXY_URL:-}"
SUMMALY_URL="${SUMMALY_URL:-}"
CSP_MEDIA_PROXY_SRC="$(endpoint_origin "$MEDIA_PROXY_URL")"
CSP_SUMMALY_SRC="$(endpoint_origin "$SUMMALY_URL")"
export MEDIA_PROXY_URL SUMMALY_URL CSP_MEDIA_PROXY_SRC CSP_SUMMALY_SRC

# Android の App Links（docs/e2ee.md の「初回設定と解錠の実装（#360）」の Android）。
# ANDROID_APP_CERT_SHA256 は、Android アプリの署名証明書の SHA-256 の指紋（`AA:BB:...` の 32 バイト。
# 鍵を替えている間は、カンマ区切りで複数）。空なら assetlinks.json を作らない（nginx は 404 を返す）。
ANDROID_APP_PACKAGE="${ANDROID_APP_PACKAGE:-works.zli.zoo}"
ANDROID_APP_CERT_SHA256="${ANDROID_APP_CERT_SHA256:-}"
well_known=/tmp/zoo-well-known
rm -rf "$well_known"
mkdir -p "$well_known"
if [ -n "$ANDROID_APP_CERT_SHA256" ]; then
  if ! printf '%s' "$ANDROID_APP_PACKAGE" | grep -Eq '^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$'; then
    echo "Invalid ANDROID_APP_PACKAGE" >&2
    exit 1
  fi
  fingerprints=""
  for fingerprint in $(printf '%s' "$ANDROID_APP_CERT_SHA256" | tr ',' ' '); do
    fingerprint="$(printf '%s' "$fingerprint" | tr 'abcdef' 'ABCDEF')"
    if ! printf '%s' "$fingerprint" | grep -Eq '^([0-9A-F]{2}:){31}[0-9A-F]{2}$'; then
      echo "Invalid ANDROID_APP_CERT_SHA256 (expected SHA-256 fingerprints like AA:BB:...)" >&2
      exit 1
    fi
    fingerprints="${fingerprints:+$fingerprints,}\"$fingerprint\""
  done
  if [ -z "$fingerprints" ]; then
    echo "Invalid ANDROID_APP_CERT_SHA256" >&2
    exit 1
  fi
  printf '[{"relation":["delegate_permission/common.handle_all_urls"],"target":{"namespace":"android_app","package_name":"%s","sha256_cert_fingerprints":[%s]}}]\n' \
    "$ANDROID_APP_PACKAGE" "$fingerprints" > "$well_known/assetlinks.json"
fi

exec /docker-entrypoint.sh "$@"
