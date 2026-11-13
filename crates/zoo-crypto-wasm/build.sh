#!/bin/sh
# zoo-crypto の wasm を作り、wasm-bindgen で JS のパッケージ（pkg/）にする。
# frontend は package.json の link: 依存でこのディレクトリを読む。
#
# wasm-pack は使わない（docs/e2ee.md の「共有の暗号コア」）。必要なのは
# wasm32-unknown-unknown の Rust と、Cargo.toml の wasm-bindgen と同じ版の wasm-bindgen-cli。
# nix develop の dev shell には両方入っている。
set -eu
cd "$(dirname "$0")"

expected="$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' Cargo.toml)"
actual="$(wasm-bindgen --version | sed -n 's/^wasm-bindgen //p')"
if [ "$expected" != "$actual" ]; then
  echo "wasm-bindgen-cli の版 ($actual) が Cargo.toml の wasm-bindgen ($expected) と違います" >&2
  exit 1
fi

cargo build --locked --release --target wasm32-unknown-unknown
rm -rf pkg
wasm-bindgen --target web --out-dir pkg \
  target/wasm32-unknown-unknown/release/zoo_crypto_wasm.wasm
