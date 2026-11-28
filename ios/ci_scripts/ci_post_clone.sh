#!/bin/bash
set -euo pipefail

# 配布元を取り違えたワークフローでは、依存の取得前に止める。
if [[ "${CI_XCODE_CLOUD:-}" != TRUE || "${CI_BRANCH:-}" != release-ios || -n "${CI_PULL_REQUEST_NUMBER:-}" || -n "${CI_TAG:-}" ]]; then
  echo 'TestFlight配布はXcode Cloudのrelease-iosブランチから実行してください。' >&2
  exit 1
fi

root="${CI_PRIMARY_REPOSITORY_PATH:?リポジトリのパスがありません。}"
cd "$root"

# Xcode Cloudはsudoを使えないため、Nixの代わりにHomebrewのrustupで同じRustを用意する。
export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
brew list --formula rustup >/dev/null 2>&1 || brew install rustup
export PATH="$(brew --prefix rustup)/bin:$PATH"
rust_version="$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)"
if [[ ! "$rust_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo 'rust-toolchain.tomlのRustバージョンを読み取れません。' >&2
  exit 1
fi

rustup toolchain install "$rust_version" --profile minimal --target aarch64-apple-ios,aarch64-apple-ios-sim
rustup run "$rust_version" bash ios/Scripts/build-crypto-xcframework.sh

# XcodeプロジェクトとPackage.resolvedは追跡済み。再生成せず、Cloudが固定版を解決する。
echo 'TestFlight配布用の暗号ライブラリを生成しました。'
