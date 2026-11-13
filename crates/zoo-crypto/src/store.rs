//! 暗号ストアに置く状態（Olm セッション、Megolm の受信用セッション、配布先の判定の記録）の
//! 保存形式（#359）。
//!
//! 端末アカウントは [`DeviceAccount::pickle`] の形式で保存する。それ以外は、この封筒に入れる。
//!
//! ```json
//! {"version": 1, "kind": "olm_session", "salt": "<32 バイト>", "nonce": "<12 バイト>", "ciphertext": "<AES-256-GCM>"}
//! ```
//!
//! - 鍵は、保存のたびに作る `salt` から `HKDF-SHA256(ikm = 保存鍵, salt, info = "zoo e2ee store key v1")`
//!   で導く（端末アカウントの pickle key と同じ考え方。同じ鍵と nonce を使い回さない）。
//! - AAD は `LP("zoo e2ee store v1") ‖ LP(kind) ‖ LP(user_id) ‖ LP(device_id)`。種類の違う値や、
//!   別のユーザーや端末の値に差し替えると、開けない。
//! - 平文は種類ごとの JSON で、vodozemac の pickle（serde の形）と、それに付く情報を入れる。

use aes_gcm::aead::{Aead, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use vodozemac::{base64_decode, base64_encode};
use zeroize::Zeroizing;

use crate::CryptoError;
use crate::device::{DeviceAccount, length_prefixed};
use crate::secret_storage::decode_fixed;

/// 封筒の版。形式を変えたら上げる。
const STORE_VERSION: u32 = 1;
/// 保存ごとの鍵を導く HKDF の info。
const STORE_KEY_INFO: &[u8] = b"zoo e2ee store key v1";
/// AAD の先頭に入れる、用途を区別する文字列。
const STORE_AAD_CONTEXT: &str = "zoo e2ee store v1";

/// 封筒に入れる値の種類。AAD に入るので、種類の違う値には差し替えられない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreKind {
    /// [`OlmSession`](crate::OlmSession)。
    OlmSession,
    /// [`InboundSessionStore`](crate::InboundSessionStore) の 1 つのキーのセッション。
    InboundSession,
    /// [`TrustStore`](crate::TrustStore)。
    TrustStore,
}

impl StoreKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::OlmSession => "olm_session",
            Self::InboundSession => "inbound_session",
            Self::TrustStore => "trust_store",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    kind: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// `plaintext` を `account` の持ち主の `kind` の値として封筒に入れる。
pub(crate) fn seal(
    storage_key: &[u8; 32],
    kind: StoreKind,
    account: &DeviceAccount,
    plaintext: &[u8],
) -> String {
    let mut salt = [0u8; 32];
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut salt).expect("OS かブラウザーの暗号用乱数を取れない");
    getrandom::fill(&mut nonce).expect("OS かブラウザーの暗号用乱数を取れない");
    let key = derive_key(storage_key, &salt);
    let aad = aad(kind, account);
    let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
        .expect("32 バイトの鍵")
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .expect("AES-GCM の暗号化は長さの上限を超えない限り失敗しない");
    let envelope = Envelope {
        version: STORE_VERSION,
        kind: kind.as_str().to_owned(),
        salt: base64_encode(salt),
        nonce: base64_encode(nonce),
        ciphertext: base64_encode(ciphertext),
    };
    serde_json::to_string(&envelope).expect("文字列と数値だけの構造体は必ず JSON にできる")
}

/// [`seal`] の封筒を開ける。
///
/// 形式が壊れている、種類が違う、保存鍵が違う、別のユーザーや端末の値に差し替えられた、の
/// どれでも [`CryptoError::InvalidPickle`] を返す（AES-GCM では区別できない）。
pub(crate) fn open(
    sealed: &str,
    storage_key: &[u8; 32],
    kind: StoreKind,
    account: &DeviceAccount,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let invalid = |message: &str| CryptoError::InvalidPickle(message.to_owned());
    let envelope: Envelope = serde_json::from_str(sealed)
        .map_err(|e| CryptoError::InvalidPickle(format!("JSON として読めません: {e}")))?;
    if envelope.version != STORE_VERSION {
        return Err(CryptoError::InvalidPickle(format!(
            "対応していない版です: {}",
            envelope.version
        )));
    }
    if envelope.kind != kind.as_str() {
        return Err(CryptoError::InvalidPickle(format!(
            "{} ではなく {} の値です",
            kind.as_str(),
            envelope.kind
        )));
    }
    let salt = decode_fixed::<32>("salt", &envelope.salt)
        .map_err(|_| invalid("salt が 32 バイトの base64 ではありません"))?;
    let nonce = decode_fixed::<12>("nonce", &envelope.nonce)
        .map_err(|_| invalid("nonce が 12 バイトの base64 ではありません"))?;
    let ciphertext = base64_decode(&envelope.ciphertext)
        .map_err(|_| invalid("ciphertext が base64 ではありません"))?;
    let key = derive_key(storage_key, &salt);
    let aad = aad(kind, account);
    Aes256Gcm::new_from_slice(key.as_ref())
        .expect("32 バイトの鍵")
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| invalid("保存鍵が違うか、別の持ち主の値か、改ざんされています"))
}

fn derive_key(storage_key: &[u8; 32], salt: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), storage_key)
        .expand(STORE_KEY_INFO, key.as_mut_slice())
        .expect("32 バイトは HKDF-SHA256 で導ける長さ（255 × 32 バイト以下）");
    key
}

fn aad(kind: StoreKind, account: &DeviceAccount) -> Vec<u8> {
    length_prefixed(&[
        STORE_AAD_CONTEXT,
        kind.as_str(),
        account.user_id(),
        account.device_id(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "6f1c1c2e-3b9a-4d4e-9f7a-1c2d3e4f5a6b";
    const DEVICE: &str = "0b7f0d3e-8c1a-4f2b-9d3e-4a5b6c7d8e9f";
    const OTHER_DEVICE: &str = "1c8e1e4f-9d2b-4a3c-8e4f-5b6c7d8e9fa0";

    #[test]
    fn round_trip_and_fresh_output() {
        let account = DeviceAccount::new(USER, DEVICE).unwrap();
        let key = [7u8; 32];
        let a = seal(&key, StoreKind::TrustStore, &account, b"hello");
        let b = seal(&key, StoreKind::TrustStore, &account, b"hello");
        assert_ne!(a, b, "保存のたびに salt と nonce が変わる");
        assert_eq!(
            open(&a, &key, StoreKind::TrustStore, &account)
                .unwrap()
                .as_slice(),
            b"hello"
        );
    }

    #[test]
    fn rejects_wrong_key_kind_and_owner() {
        let account = DeviceAccount::new(USER, DEVICE).unwrap();
        let other = DeviceAccount::new(USER, OTHER_DEVICE).unwrap();
        let sealed = seal(&[1u8; 32], StoreKind::OlmSession, &account, b"x");
        for result in [
            open(&sealed, &[2u8; 32], StoreKind::OlmSession, &account),
            open(&sealed, &[1u8; 32], StoreKind::TrustStore, &account),
            open(&sealed, &[1u8; 32], StoreKind::OlmSession, &other),
        ] {
            assert!(matches!(result, Err(CryptoError::InvalidPickle(_))));
        }
        // kind を書き換えても AAD が合わないので開けない。
        let forged = sealed.replace("olm_session", "trust_store");
        assert!(matches!(
            open(&forged, &[1u8; 32], StoreKind::TrustStore, &account),
            Err(CryptoError::InvalidPickle(_))
        ));
    }
}
