//! zoo-crypto（`crates/zoo-crypto`）の UniFFI バインディング（#353）。仕様は `docs/e2ee.md` にある。
//!
//! Android（`frontend-compose` の androidMain）から Kotlin で使う。型の対応は次のとおり。
//!
//! - 状態を持つもの（端末アカウント、Olm と Megolm のセッション、配布先の判定の記録など）は
//!   UniFFI のオブジェクトにする。中身は `Mutex` で守るので、どのスレッドから呼んでもよい。
//! - サーバーとやり取りする JSON（端末鍵、to-device の `content`、`ciphertext` など）は、
//!   コアと同じ形の JSON 文字列で受け渡す。Kotlin 側で型を二重に持たないため。
//! - 鍵などのバイト列は `ByteArray`、公開鍵はパディングなしの base64 の文字列にする。
//!
//! 秘密（保存鍵、シード、リカバリーキーなど）を Kotlin に返すと、JVM の `String` や `ByteArray` は
//! Rust の `Zeroizing` のように消せない。返すのは、利用者に見せる、SSSS に入れる、など
//! Kotlin で使う必要があるものだけにする。

mod backup;
mod cross_signing;
mod device;
mod error;
mod megolm;
mod secret_storage;
mod setup;
mod trust;

pub use backup::*;
pub use cross_signing::*;
pub use device::*;
pub use error::ZooCryptoError;
pub use megolm::*;
pub use secret_storage::*;
pub use setup::*;
pub use trust::*;

uniffi::setup_scaffolding!();

pub(crate) type Result<T, E = ZooCryptoError> = std::result::Result<T, E>;

/// JSON 文字列を読む。
pub(crate) fn parse_json(field: &str, json: &str) -> Result<serde_json::Value> {
    serde_json::from_str(json).map_err(|e| ZooCryptoError::InvalidPayload {
        reason: format!("{field} が JSON ではありません: {e}"),
    })
}

/// 32 バイトの鍵を受け取る。
pub(crate) fn bytes32(field: &str, bytes: &[u8]) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| ZooCryptoError::InvalidPayload {
            reason: format!(
                "{field} は 32 バイトでなければなりません（{} バイト）",
                bytes.len()
            ),
        })
}

/// コアのロックを取る。ほかのスレッドが panic してロックが壊れても、状態は使い続ける
/// （コアの操作は、失敗したら状態を変えないか、途中まで変えても壊れた状態にはしない）。
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
