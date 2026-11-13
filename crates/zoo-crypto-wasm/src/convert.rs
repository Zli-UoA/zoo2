//! JS の値と Rust の値の変換、エラーの変換。

use js_sys::{Error, JSON, Reflect};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;
use zoo_crypto::{CryptoError, Ed25519PublicKey, Rejection};

/// JS に返すエラーの `name`。
pub(crate) const ERROR_NAME: &str = "ZooCryptoError";

/// `name`、`message`、`code`（と `rejection`）を持つ JS の `Error` を作る。
fn error(code: &str, message: &str) -> Error {
    let error = Error::new(message);
    error.set_name(ERROR_NAME);
    let _ = Reflect::set(&error, &"code".into(), &code.into());
    error
}

/// 引数が正しくないときのエラー（`code` は `InvalidArgument`）。
pub(crate) fn invalid_argument(message: impl AsRef<str>) -> JsValue {
    error("InvalidArgument", message.as_ref()).into()
}

/// [`CryptoError`] を JS の `Error` にする。`code` に列挙子の名前を入れる。
/// 受信時の検証で破棄したとき（`Rejected`）は、`rejection` に理由の名前も入れる。
pub(crate) fn crypto_error(e: CryptoError) -> JsValue {
    let code = match &e {
        CryptoError::InvalidId { .. } => "InvalidId",
        CryptoError::CanonicalJson(_) => "CanonicalJson",
        CryptoError::Key(_) => "Key",
        CryptoError::InvalidDeviceKeys(_) => "InvalidDeviceKeys",
        CryptoError::MissingSignature { .. } => "MissingSignature",
        CryptoError::Signature(_) => "Signature",
        CryptoError::Pickle(_) => "Pickle",
        CryptoError::InvalidPickle(_) => "InvalidPickle",
        CryptoError::PickleOwnerMismatch { .. } => "PickleOwnerMismatch",
        CryptoError::InvalidPayload(_) => "InvalidPayload",
        CryptoError::Olm(_) => "Olm",
        CryptoError::Megolm(_) => "Megolm",
        CryptoError::MissingRoomKey { .. } => "MissingRoomKey",
        CryptoError::InvalidCrossSigningKey(_) => "InvalidCrossSigningKey",
        CryptoError::InvalidTrustStore(_) => "InvalidTrustStore",
        CryptoError::UserSigningKeyRollback(_) => "UserSigningKeyRollback",
        CryptoError::StaleUserSigningKey(_) => "StaleUserSigningKey",
        CryptoError::InvalidRecoveryKey(_) => "InvalidRecoveryKey",
        CryptoError::InvalidSecretStorage(_) => "InvalidSecretStorage",
        CryptoError::NoKeySlot => "NoKeySlot",
        CryptoError::KeySlotUnlock => "KeySlotUnlock",
        CryptoError::SsssKeyMismatch => "SsssKeyMismatch",
        CryptoError::SecretDecryption(_) => "SecretDecryption",
        CryptoError::InvalidBackup(_) => "InvalidBackup",
        CryptoError::BackupKeyMismatch => "BackupKeyMismatch",
        CryptoError::BackupVersionRollback { .. } => "BackupVersionRollback",
        CryptoError::BackupDecryption(_) => "BackupDecryption",
        CryptoError::Rejected(_) => "Rejected",
        CryptoError::ServerKeyMismatch(_) => "ServerKeyMismatch",
        _ => "Unknown",
    };
    let error = error(code, &e.to_string());
    if let CryptoError::Rejected(rejection) = &e {
        let _ = Reflect::set(
            &error,
            &"rejection".into(),
            &rejection_code(rejection).into(),
        );
    }
    error.into()
}

fn rejection_code(rejection: &Rejection) -> &'static str {
    match rejection {
        Rejection::InvalidSenderDeviceKeys(_) => "InvalidSenderDeviceKeys",
        Rejection::SenderDeviceKeysConflict => "SenderDeviceKeysConflict",
        Rejection::UnknownSenderDevice => "UnknownSenderDevice",
        Rejection::SenderKeyMismatch => "SenderKeyMismatch",
        Rejection::SenderSigningKeyMismatch => "SenderSigningKeyMismatch",
        Rejection::RecipientMismatch => "RecipientMismatch",
        Rejection::ChannelMismatch => "ChannelMismatch",
        Rejection::MessageIdMismatch => "MessageIdMismatch",
        Rejection::ParentMismatch => "ParentMismatch",
        Rejection::KindMismatch => "KindMismatch",
        Rejection::SenderMismatch => "SenderMismatch",
        Rejection::DeviceMismatch => "DeviceMismatch",
        Rejection::EpochMismatch => "EpochMismatch",
        Rejection::Replay { .. } => "Replay",
        Rejection::RevisionMismatch => "RevisionMismatch",
        _ => "Unknown",
    }
}

/// `Result<T, CryptoError>` を `Result<T, JsValue>` にする。
pub(crate) trait IntoJs<T> {
    fn js(self) -> Result<T, JsValue>;
}

impl<T> IntoJs<T> for Result<T, CryptoError> {
    fn js(self) -> Result<T, JsValue> {
        self.map_err(crypto_error)
    }
}

/// JS の値を、`JSON.stringify` を通して Rust の値にする。
///
/// serde-wasm-bindgen を使わないのは、JSON と同じ規則（整数を整数のまま、`undefined` の項目を
/// 落とす）で読むため。Canonical JSON は小数を拒むので、整数が `1.0` になると署名が合わない。
pub(crate) fn from_js<T: DeserializeOwned>(value: &JsValue, what: &str) -> Result<T, JsValue> {
    let json = JSON::stringify(value)
        .map_err(|_| invalid_argument(format!("{what} を JSON にできません")))?;
    let json = JsValue::from(json)
        .as_string()
        .ok_or_else(|| invalid_argument(format!("{what} がありません")))?;
    serde_json::from_str(&json)
        .map_err(|e| invalid_argument(format!("{what} の形式が正しくありません: {e}")))
}

/// 省略できる引数（`null` と `undefined` は `None`）。
pub(crate) fn from_js_opt<T: DeserializeOwned>(
    value: &JsValue,
    what: &str,
) -> Result<Option<T>, JsValue> {
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        from_js(value, what).map(Some)
    }
}

/// Rust の値を、JSON を通して JS の値にする。
pub(crate) fn to_js<T: Serialize + ?Sized>(value: &T) -> Result<JsValue, JsValue> {
    let json = serde_json::to_string(value)
        .map_err(|e| invalid_argument(format!("JSON にできません: {e}")))?;
    JSON::parse(&json)
}

/// JS のオブジェクトに項目を足す。
pub(crate) fn set(target: &JsValue, key: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(target, &key.into(), value).map(|_| ())
}

/// 32 バイトの鍵やシードを受け取る。呼び出しのあとで消せるよう [`Zeroizing`] で包む。
pub(crate) fn bytes32(bytes: &[u8], what: &str) -> Result<Zeroizing<[u8; 32]>, JsValue> {
    let mut out = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return Err(invalid_argument(format!(
            "{what} は 32 バイトでなければなりません（{} バイト）",
            bytes.len()
        )));
    }
    out.copy_from_slice(bytes);
    Ok(out)
}

/// JS の数を、JS で正確に表せる 0 以上の整数（`Number.MAX_SAFE_INTEGER` まで）として受け取る。
pub(crate) fn safe_u64(value: f64, what: &str) -> Result<u64, JsValue> {
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= 9_007_199_254_740_991.0
    {
        Ok(value as u64)
    } else {
        Err(invalid_argument(format!(
            "{what} は 0 から Number.MAX_SAFE_INTEGER までの整数でなければなりません"
        )))
    }
}

/// 省略できる整数。
pub(crate) fn safe_u64_opt(value: Option<f64>, what: &str) -> Result<Option<u64>, JsValue> {
    value.map(|v| safe_u64(v, what)).transpose()
}

/// Ed25519 の公開鍵（パディングなしの base64）を読む。
pub(crate) fn ed25519(key: &str) -> Result<Ed25519PublicKey, JsValue> {
    Ed25519PublicKey::from_base64(key).map_err(|e| crypto_error(e.into()))
}
