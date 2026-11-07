//! zoo2 の E2EE の暗号コア。仕様は `docs/e2ee.md` にある。
//!
//! vodozemac 0.11 の上に、全クライアント（React、Compose、TUI）で共有する処理を置く。
//! このクレートは I/O を持たない。サーバーの JSON を受け取り、送る JSON や保存する状態を返す。
//! 保存先と通信は、各プラットフォームのバインディングと呼び出し側が実装する。
//!
//! 今あるもの（#348）:
//!
//! - [`DeviceAccount`]：端末の識別鍵、ワンタイム鍵、フォールバック鍵と、署名付きの公開鍵の書き出し
//! - [`DeviceAccount::pickle`] と [`DeviceAccount::from_pickle`]：呼び出し側の 32 バイトの鍵で暗号化した保存と復元
//! - [`to_canonical_json`]、[`verify_json_signature`]、[`verify_device_keys`]：Matrix と同じ Canonical JSON と署名の検証

mod canonical_json;
mod device;
mod error;
mod signing;

pub use canonical_json::to_canonical_json;
pub use device::{
    DeviceAccount, DeviceKeys, MEGOLM_ALGORITHM, OLM_ALGORITHM, SIGNED_CURVE25519,
    verify_device_keys,
};
pub use error::CryptoError;
pub use signing::{signable_json, verify_json_signature};
pub use vodozemac::olm::IdentityKeys;
pub use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, PickleError};
