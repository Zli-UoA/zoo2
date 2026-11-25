//! `cargo run --features cli --bin uniffi-bindgen -- generate --library <.so> --language kotlin ...`
//! （Swift は `--language swift`。iOS は ios/Scripts/build-crypto-xcframework.sh）
fn main() {
    uniffi::uniffi_bindgen_main()
}
