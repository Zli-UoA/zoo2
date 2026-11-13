//! `cargo run --features cli --bin uniffi-bindgen -- generate --library <.so> --language kotlin ...`
fn main() {
    uniffi::uniffi_bindgen_main()
}
