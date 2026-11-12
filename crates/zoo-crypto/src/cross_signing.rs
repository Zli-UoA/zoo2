//! クロス署名の鍵（マスター鍵、自己署名鍵、ユーザー署名鍵）と、その署名と検証。
//!
//! 形式は docs/e2ee.md の「クロス署名」と同じ（Matrix の
//! [Cross-signing](https://spec.matrix.org/latest/client-server-api/#cross-signing) に合わせる）。
//!
//! ```json
//! {"user_id": "<user_id>", "usage": ["master"], "keys": {"ed25519:<公開鍵>": "<公開鍵>"}, "signatures": {}}
//! ```
//!
//! - 自己署名鍵とユーザー署名鍵には、マスター鍵で署名する。
//! - 自分の端末鍵には、自己署名鍵で署名する。
//! - 他のユーザーのマスター鍵と TOFU のピン（`zoo.tofu_pin`）には、ユーザー署名鍵で署名する。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vodozemac::{Ed25519PublicKey, Ed25519SecretKey};
use zeroize::{Zeroize, Zeroizing};

use crate::device::validate_uuid;
use crate::signing::{sign_json_with, signable_json};
use crate::{CryptoError, verify_device_keys, verify_json_signature};

/// TOFU のピンの `type`。
pub const TOFU_PIN_TYPE: &str = "zoo.tofu_pin";

/// クロス署名鍵の用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyUsage {
    /// マスター鍵。ユーザーの身元で、自己署名鍵とユーザー署名鍵に署名する。
    Master,
    /// 自己署名鍵。自分の端末鍵に署名する。
    SelfSigning,
    /// ユーザー署名鍵。他のユーザーのマスター鍵と TOFU のピンに署名する。
    UserSigning,
}

impl KeyUsage {
    /// JSON の `usage` に入れる名前（`master`、`self_signing`、`user_signing`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::SelfSigning => "self_signing",
            Self::UserSigning => "user_signing",
        }
    }

    /// 秘密鍵を SSSS に置くときの秘密の名前（`m.cross_signing.<用途>`）。
    pub fn secret_name(self) -> &'static str {
        match self {
            Self::Master => "m.cross_signing.master",
            Self::SelfSigning => "m.cross_signing.self_signing",
            Self::UserSigning => "m.cross_signing.user_signing",
        }
    }
}

/// クロス署名鍵の鍵 ID（`ed25519:<公開鍵>`）。
pub fn cross_signing_key_id(public_key: &Ed25519PublicKey) -> String {
    format!("ed25519:{}", public_key.to_base64())
}

/// `POST /api/e2ee/cross_signing/signatures` の `signatures` の 1 件。署名者は常に呼び出したユーザー。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureUpload {
    /// 署名の対象の持ち主。
    pub target_user_id: String,
    /// 対象の ID。端末なら `device_id`、マスター鍵なら公開鍵（`ed25519:` を付けない）。
    pub target_key_id: String,
    /// 署名した鍵の鍵 ID（`ed25519:<公開鍵>`）。
    pub signer_key_id: String,
    /// 署名（パディングなしの base64）。
    pub signature: String,
}

/// `keys/query` と配布先のスナップショットの `signatures` の 1 行（`e2ee_key_signatures`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySignature {
    /// 署名したユーザー。
    pub signer_user_id: String,
    /// 署名した鍵の鍵 ID（`ed25519:<公開鍵>`）。
    pub signer_key_id: String,
    /// 署名の対象の持ち主。
    pub target_user_id: String,
    /// 対象の ID。端末なら `device_id`、マスター鍵なら公開鍵。
    pub target_key_id: String,
    /// 署名（パディングなしの base64）。
    pub signature: String,
}

/// クロス署名鍵の秘密鍵 1 つ。
///
/// 秘密鍵は Ed25519 の 32 バイトのシードで、SSSS に置く（#351）。`Debug` は公開鍵だけを出す。
pub struct CrossSigningKeyPair {
    user_id: String,
    usage: KeyUsage,
    secret: Ed25519SecretKey,
}

impl CrossSigningKeyPair {
    /// 新しい鍵を作る。`user_id` は小文字でハイフン区切りの UUID でなければならない。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate(user_id: &str, usage: KeyUsage) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        Ok(Self {
            user_id: user_id.to_owned(),
            usage,
            secret: Ed25519SecretKey::new(),
        })
    }

    /// SSSS から取り出した 32 バイトのシードから戻す。
    pub fn from_seed(user_id: &str, usage: KeyUsage, seed: &[u8; 32]) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        Ok(Self {
            user_id: user_id.to_owned(),
            usage,
            secret: Ed25519SecretKey::from_slice(seed),
        })
    }

    /// SSSS に置く 32 バイトのシード。
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        let mut bytes = self.secret.to_bytes();
        let seed = Zeroizing::new(*bytes);
        bytes.zeroize();
        seed
    }

    /// 持ち主のユーザー ID。
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// 用途。
    pub fn usage(&self) -> KeyUsage {
        self.usage
    }

    /// 公開鍵。
    pub fn public_key(&self) -> Ed25519PublicKey {
        self.secret.public_key()
    }

    /// 鍵 ID（`ed25519:<公開鍵>`）。
    pub fn key_id(&self) -> String {
        cross_signing_key_id(&self.public_key())
    }

    /// 署名のない公開鍵の JSON（`{"user_id", "usage", "keys"}`）。
    pub fn public_key_json(&self) -> Value {
        let public_key = self.public_key().to_base64();
        json!({
            "user_id": self.user_id,
            "usage": [self.usage.as_str()],
            "keys": { format!("ed25519:{public_key}"): public_key },
        })
    }

    /// マスター鍵で、同じユーザーの自己署名鍵かユーザー署名鍵に署名した公開鍵の JSON を返す。
    /// `POST /api/e2ee/cross_signing/keys` の `self_signing_key` と `user_signing_key` に入れる。
    pub fn sign_subkey(&self, subkey: &CrossSigningKeyPair) -> Result<Value, CryptoError> {
        self.require(KeyUsage::Master)?;
        if subkey.usage == KeyUsage::Master {
            return Err(CryptoError::InvalidCrossSigningKey(
                "マスター鍵が署名するのは自己署名鍵とユーザー署名鍵だけです".to_owned(),
            ));
        }
        if subkey.user_id != self.user_id {
            return Err(CryptoError::InvalidCrossSigningKey(
                "別のユーザーの鍵にはマスター鍵で署名できません".to_owned(),
            ));
        }
        let mut value = subkey.public_key_json();
        sign_json_with(&mut value, &self.user_id, &self.key_id(), |m| {
            self.secret.sign(m)
        })?;
        Ok(value)
    }

    /// 自己署名鍵で、自分の端末鍵に署名する。
    ///
    /// 端末鍵は [`verify_device_keys`] で検証してから署名する（持ち主が違う、自己署名が壊れている
    /// 端末鍵には署名しない）。
    pub fn sign_device(&self, device_keys: &Value) -> Result<SignatureUpload, CryptoError> {
        self.require(KeyUsage::SelfSigning)?;
        let device_id = device_keys
            .get("device_id")
            .and_then(Value::as_str)
            .ok_or_else(|| CryptoError::InvalidDeviceKeys("device_id がありません".to_owned()))?;
        verify_device_keys(device_keys, &self.user_id, device_id)?;
        Ok(SignatureUpload {
            target_user_id: self.user_id.clone(),
            target_key_id: device_id.to_owned(),
            signer_key_id: self.key_id(),
            signature: self.signature_over(device_keys)?,
        })
    }

    /// ユーザー署名鍵で、他のユーザーのマスター鍵に署名する（検証済みの印）。
    ///
    /// 署名するのは、変わったマスター鍵を利用者が明示的に受け入れたとき（#362）と、SAS で
    /// 検証したとき（#368）だけ。送るときは、同じ操作で `replace: true` のピンも作る
    /// （[`TrustStore::accept_master_key`](crate::TrustStore::accept_master_key)）。
    pub fn sign_master_key(
        &self,
        target_user_id: &str,
        master_key: &Value,
    ) -> Result<SignatureUpload, CryptoError> {
        self.require(KeyUsage::UserSigning)?;
        self.require_other_user(target_user_id)?;
        let target = verify_cross_signing_key(master_key, target_user_id, KeyUsage::Master)?;
        Ok(SignatureUpload {
            target_user_id: target_user_id.to_owned(),
            target_key_id: target.to_base64(),
            signer_key_id: self.key_id(),
            signature: self.signature_over(master_key)?,
        })
    }

    /// ユーザー署名鍵で、TOFU のピン（`zoo.tofu_pin`）を作る。
    ///
    /// ```json
    /// {"type": "zoo.tofu_pin", "user_id": "<自分>", "target_user_id": "<相手>", "master_key": "<相手のマスター鍵>", "signatures": {}}
    /// ```
    pub fn sign_tofu_pin(
        &self,
        target_user_id: &str,
        master_key: &Ed25519PublicKey,
    ) -> Result<Value, CryptoError> {
        self.require(KeyUsage::UserSigning)?;
        self.require_other_user(target_user_id)?;
        let mut pin = json!({
            "type": TOFU_PIN_TYPE,
            "user_id": self.user_id,
            "target_user_id": target_user_id,
            "master_key": master_key.to_base64(),
        });
        sign_json_with(&mut pin, &self.user_id, &self.key_id(), |m| {
            self.secret.sign(m)
        })?;
        Ok(pin)
    }

    /// 鍵バックアップの版の `auth_data` に署名する（マスター鍵だけ）。
    pub(crate) fn sign_auth_data(&self, auth_data: &mut Value) -> Result<(), CryptoError> {
        self.require(KeyUsage::Master)?;
        sign_json_with(auth_data, &self.user_id, &self.key_id(), |m| {
            self.secret.sign(m)
        })
    }

    fn signature_over(&self, value: &Value) -> Result<String, CryptoError> {
        let message = signable_json(value)?;
        Ok(self.secret.sign(message.as_bytes()).to_base64())
    }

    fn require(&self, usage: KeyUsage) -> Result<(), CryptoError> {
        if self.usage == usage {
            Ok(())
        } else {
            Err(CryptoError::InvalidCrossSigningKey(format!(
                "{} の鍵では {} の鍵の操作をできません",
                self.usage.as_str(),
                usage.as_str()
            )))
        }
    }

    fn require_other_user(&self, target_user_id: &str) -> Result<(), CryptoError> {
        validate_uuid("target_user_id", target_user_id, false)?;
        if target_user_id == self.user_id {
            return Err(CryptoError::InvalidCrossSigningKey(
                "自分のマスター鍵にはユーザー署名鍵で署名しません".to_owned(),
            ));
        }
        Ok(())
    }
}

impl std::fmt::Debug for CrossSigningKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrossSigningKeyPair")
            .field("user_id", &self.user_id)
            .field("usage", &self.usage)
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

/// 1 人のユーザーのクロス署名鍵 3 つ。初回設定（#360）と失効の手順（#362）で作る。
#[derive(Debug)]
pub struct CrossSigningKeys {
    /// マスター鍵。
    pub master: CrossSigningKeyPair,
    /// 自己署名鍵。
    pub self_signing: CrossSigningKeyPair,
    /// ユーザー署名鍵。
    pub user_signing: CrossSigningKeyPair,
}

impl CrossSigningKeys {
    /// 3 つの鍵を新しく作る。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate(user_id: &str) -> Result<Self, CryptoError> {
        Ok(Self {
            master: CrossSigningKeyPair::generate(user_id, KeyUsage::Master)?,
            self_signing: CrossSigningKeyPair::generate(user_id, KeyUsage::SelfSigning)?,
            user_signing: CrossSigningKeyPair::generate(user_id, KeyUsage::UserSigning)?,
        })
    }

    /// `POST /api/e2ee/cross_signing/keys` の `master_key`、`self_signing_key`、`user_signing_key`。
    /// 自己署名鍵とユーザー署名鍵には、マスター鍵の署名を付ける。
    pub fn upload_json(&self) -> Value {
        let sign = |subkey| {
            self.master
                .sign_subkey(subkey)
                .expect("generate で作った同じユーザーの鍵には必ず署名できる")
        };
        json!({
            "master_key": self.master.public_key_json(),
            "self_signing_key": sign(&self.self_signing),
            "user_signing_key": sign(&self.user_signing),
        })
    }
}

/// サーバーから受け取ったクロス署名鍵の JSON を検証し、公開鍵を返す。
///
/// `user_id` が期待した値で、`usage` が `[<用途>]` だけで、`keys` がちょうど 1 つの
/// `ed25519:<公開鍵>` で、鍵 ID と値が同じことを確かめる。署名は確かめない（誰が署名したかは
/// 呼び出し側で決まるため）。
pub fn verify_cross_signing_key(
    value: &Value,
    user_id: &str,
    usage: KeyUsage,
) -> Result<Ed25519PublicKey, CryptoError> {
    let invalid = |message: String| CryptoError::InvalidCrossSigningKey(message);
    if value.get("user_id").and_then(Value::as_str) != Some(user_id) {
        return Err(invalid(format!("user_id が {user_id} ではありません")));
    }
    let usages = value.get("usage").and_then(Value::as_array);
    if usages.map(Vec::as_slice) != Some(&[Value::String(usage.as_str().to_owned())]) {
        return Err(invalid(format!(
            "usage が [{}] ではありません",
            usage.as_str()
        )));
    }
    let keys = value
        .get("keys")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("keys がありません".to_owned()))?;
    let mut entries = keys.iter();
    let (Some((key_id, key)), None) = (entries.next(), entries.next()) else {
        return Err(invalid("keys の鍵は 1 つでなければなりません".to_owned()));
    };
    let key = key
        .as_str()
        .ok_or_else(|| invalid("keys の値が文字列ではありません".to_owned()))?;
    let public_key = Ed25519PublicKey::from_base64(key)?;
    if *key_id != cross_signing_key_id(&public_key) {
        return Err(invalid(format!("鍵 ID {key_id} が鍵と一致しません")));
    }
    Ok(public_key)
}

/// 署名を確かめた結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureCheck {
    /// 正しい署名がある。
    Valid,
    /// 署名がない。
    Missing,
    /// 署名はあるが、壊れているか一致しない。
    Invalid,
}

/// `target`（`target_user_id` の `target_key_id`）に、`signer_user_id` の `signer` の署名があるかを確かめる。
///
/// 署名は、JSON の `signatures` に入っているもの（クロス署名鍵のアップロード）と、
/// `e2ee_key_signatures` の行（`POST /api/e2ee/cross_signing/signatures`）のどちらでもよい。
pub(crate) fn check_signature(
    target: &Value,
    target_user_id: &str,
    target_key_id: &str,
    signer_user_id: &str,
    signer: &Ed25519PublicKey,
    rows: &[KeySignature],
) -> SignatureCheck {
    let signer_key_id = cross_signing_key_id(signer);
    let mut result = match verify_json_signature(target, signer_user_id, &signer_key_id, signer) {
        Ok(()) => return SignatureCheck::Valid,
        Err(CryptoError::MissingSignature { .. }) => SignatureCheck::Missing,
        Err(_) => SignatureCheck::Invalid,
    };
    let Ok(message) = signable_json(target) else {
        return SignatureCheck::Invalid;
    };
    for row in rows.iter().filter(|row| {
        row.signer_user_id == signer_user_id
            && row.signer_key_id == signer_key_id
            && row.target_user_id == target_user_id
            && row.target_key_id == target_key_id
    }) {
        let valid = vodozemac::Ed25519Signature::from_base64(&row.signature)
            .is_ok_and(|signature| signer.verify(message.as_bytes(), &signature).is_ok());
        if valid {
            return SignatureCheck::Valid;
        }
        result = SignatureCheck::Invalid;
    }
    result
}
