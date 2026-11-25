#!/usr/bin/env bash
# crates/zoo-crypto-ffi から、iOS 向けの XCFramework と Swift のバインディングを作る（#685）。
# 使い方: `nix develop .#ios -c make crypto-xcframework`（ios/ から、またはルートの `make crypto-ffi-ios`）。
# 生成物（ios/ZooCrypto/）は git に入れない。Android の .so と同じで、ビルドのたびに作る。
# バインディングはアプリと同じモジュールに入れると型名（MessageContent など）が衝突するので、
# ローカルの Swift パッケージ `ZooCrypto`（`import ZooCrypto`）にする。
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
crate="$root/crates/zoo-crypto-ffi"
out="$root/ios/ZooCrypto"
# flake.nix の iosRustTargets と同じ。watchOS と x86_64 のシミュレーターは対象外。
IOS_TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
LIB=libzoo_crypto_ffi.a

# Nix のシェルが入れる DEVELOPER_DIR・SDKROOT は macOS 用の SDK なので外し、`xcode-select` の Xcode を使う。
# Nix の cc が iOS の SDK を見失わないように、各 target で Xcode の clang と SDK を指す。
unset DEVELOPER_DIR SDKROOT
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-26.0}"

rm -rf "$out"
mkdir -p "$out/headers" "$out/Sources/ZooCrypto"

for target in "${IOS_TARGETS[@]}"; do
  case "$target" in
    *-sim) sdk=iphonesimulator ;;
    *) sdk=iphoneos ;;
  esac
  env_target="$(echo "$target" | tr '-' '_')"
  upper="$(echo "$env_target" | tr '[:lower:]' '[:upper:]')"
  sdkroot="$(/usr/bin/xcrun --sdk "$sdk" --show-sdk-path)"
  clang="$(/usr/bin/xcrun --sdk "$sdk" --find clang)"
  (
    export SDKROOT="$sdkroot"
    export "CC_${env_target}=$clang"
    export "CARGO_TARGET_${upper}_LINKER=$clang"
    cd "$crate"
    # 出力先を固定する（`CARGO_TARGET_DIR` が設定されていても、あとの処理が今回の成果物を読む）。
    cargo build --locked --release --lib --target "$target" --target-dir "$crate/target"
  )
done

# Swift のバインディングは、ビルドしたライブラリーから作る（版が必ず一致する）。
first="${IOS_TARGETS[0]}"
(
  cd "$crate"
  cargo run --locked --features cli --target-dir "$crate/target" --bin uniffi-bindgen -- generate \
    --library "target/$first/release/$LIB" --language swift --out-dir "$out/generated"
)
cp "$out"/generated/*.swift "$out/Sources/ZooCrypto/"
cp "$out"/generated/*.h "$out/headers/"
# modulemap は xcframework の headers に `module.modulemap` で置く。
cp "$out"/generated/*.modulemap "$out/headers/module.modulemap"

args=()
for target in "${IOS_TARGETS[@]}"; do
  args+=(-library "$crate/target/$target/release/$LIB" -headers "$out/headers")
done
/usr/bin/xcrun xcodebuild -create-xcframework "${args[@]}" -output "$out/ZooCryptoFFI.xcframework"
cat > "$out/Package.swift" <<'SWIFT'
// swift-tools-version: 6.0
// build-crypto-xcframework.sh が作る。手で編集しない。
import PackageDescription

let package = Package(
    name: "ZooCrypto",
    platforms: [.iOS("26.0")],
    products: [.library(name: "ZooCrypto", targets: ["ZooCrypto"])],
    targets: [
        .binaryTarget(name: "ZooCryptoFFIFFI", path: "ZooCryptoFFI.xcframework"),
        .target(
            name: "ZooCrypto",
            dependencies: ["ZooCryptoFFIFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
SWIFT
echo "作成しました: ${out}（Swift パッケージ ZooCrypto）"
