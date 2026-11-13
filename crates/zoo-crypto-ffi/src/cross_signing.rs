//! クロス署名（#350）。

use std::sync::Arc;

use zoo_crypto as core;

use crate::{Result, bytes32, ed25519_from_base64, parse_json};

/// クロス署名鍵の用途（[`core::KeyUsage`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeyUsage {
    Master,
    SelfSigning,
    UserSigning,
}

impl From<KeyUsage> for core::KeyUsage {
    fn from(usage: KeyUsage) -> Self {
        match usage {
            KeyUsage::Master => Self::Master,
            KeyUsage::SelfSigning => Self::SelfSigning,
            KeyUsage::UserSigning => Self::UserSigning,
        }
    }
}

impl From<core::KeyUsage> for KeyUsage {
    fn from(usage: core::KeyUsage) -> Self {
        match usage {
            core::KeyUsage::Master => Self::Master,
            core::KeyUsage::SelfSigning => Self::SelfSigning,
            core::KeyUsage::UserSigning => Self::UserSigning,
        }
    }
}

/// `POST /api/e2ee/cross_signing/signatures` の `signatures` の 1 件（[`core::SignatureUpload`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SignatureUpload {
    pub target_user_id: String,
    pub target_key_id: String,
    pub signer_key_id: String,
    pub signature: String,
}

impl From<core::SignatureUpload> for SignatureUpload {
    fn from(s: core::SignatureUpload) -> Self {
        Self {
            target_user_id: s.target_user_id,
            target_key_id: s.target_key_id,
            signer_key_id: s.signer_key_id,
            signature: s.signature,
        }
    }
}

/// クロス署名鍵の秘密鍵 1 つ（[`core::CrossSigningKeyPair`]）。
#[derive(uniffi::Object)]
pub struct CrossSigningKeyPair {
    pub(crate) inner: core::CrossSigningKeyPair,
}

impl CrossSigningKeyPair {
    pub(crate) fn wrap(inner: core::CrossSigningKeyPair) -> Arc<Self> {
        Arc::new(Self { inner })
    }

    /// 同じ鍵をもう 1 つ作る（コアの鍵は `Clone` を持たないので、シードから作り直す）。
    pub(crate) fn duplicate(key: &core::CrossSigningKeyPair) -> core::CrossSigningKeyPair {
        core::CrossSigningKeyPair::from_seed(key.user_id(), key.usage(), &key.seed())
            .expect("正しい鍵から取り出したユーザー ID とシードからは必ず作れる")
    }
}

#[uniffi::export]
impl CrossSigningKeyPair {
    #[uniffi::constructor]
    pub fn generate(user_id: String, usage: KeyUsage) -> Result<Arc<Self>> {
        Ok(Self::wrap(core::CrossSigningKeyPair::generate(
            &user_id,
            usage.into(),
        )?))
    }

    /// 32 バイトのシードから戻す。
    #[uniffi::constructor]
    pub fn from_seed(user_id: String, usage: KeyUsage, seed: Vec<u8>) -> Result<Arc<Self>> {
        let seed = zeroize::Zeroizing::new(bytes32("seed", &seed)?);
        Ok(Self::wrap(core::CrossSigningKeyPair::from_seed(
            &user_id,
            usage.into(),
            &seed,
        )?))
    }

    /// 32 バイトのシード（秘密）。
    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    pub fn user_id(&self) -> String {
        self.inner.user_id().to_owned()
    }

    pub fn usage(&self) -> KeyUsage {
        self.inner.usage().into()
    }

    /// 公開鍵（base64）。
    pub fn public_key(&self) -> String {
        self.inner.public_key().to_base64()
    }

    /// 鍵 ID（`ed25519:<公開鍵>`）。
    pub fn key_id(&self) -> String {
        self.inner.key_id()
    }

    /// 署名なしの公開鍵の JSON。
    pub fn public_key_json(&self) -> String {
        self.inner.public_key_json().to_string()
    }

    /// 自己署名鍵かユーザー署名鍵に、このマスター鍵で署名した JSON。
    pub fn sign_subkey(&self, subkey: &CrossSigningKeyPair) -> Result<String> {
        Ok(self.inner.sign_subkey(&subkey.inner)?.to_string())
    }

    /// 自分の端末鍵（JSON）に自己署名鍵で署名する。
    pub fn sign_device(&self, device_keys: String) -> Result<SignatureUpload> {
        let value = parse_json("device_keys", &device_keys)?;
        Ok(self.inner.sign_device(&value)?.into())
    }

    /// 他のユーザーのマスター鍵（JSON）にユーザー署名鍵で署名する。
    pub fn sign_master_key(
        &self,
        target_user_id: String,
        master_key: String,
    ) -> Result<SignatureUpload> {
        let value = parse_json("master_key", &master_key)?;
        Ok(self.inner.sign_master_key(&target_user_id, &value)?.into())
    }

    /// TOFU のピン（`zoo.tofu_pin`、JSON）にユーザー署名鍵で署名する。`master_key` は base64。
    pub fn sign_tofu_pin(&self, target_user_id: String, master_key: String) -> Result<String> {
        let master_key = ed25519_from_base64(&master_key)?;
        Ok(self
            .inner
            .sign_tofu_pin(&target_user_id, &master_key)?
            .to_string())
    }
}

/// 1 人のユーザーのクロス署名鍵 3 つ（[`core::CrossSigningKeys`]）。
#[derive(uniffi::Object)]
pub struct CrossSigningKeys {
    pub(crate) inner: core::CrossSigningKeys,
}

#[uniffi::export]
impl CrossSigningKeys {
    /// 3 つの鍵を新しく作る。
    #[uniffi::constructor]
    pub fn generate(user_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: core::CrossSigningKeys::generate(&user_id)?,
        }))
    }

    pub fn master(&self) -> Arc<CrossSigningKeyPair> {
        CrossSigningKeyPair::wrap(CrossSigningKeyPair::duplicate(&self.inner.master))
    }

    pub fn self_signing(&self) -> Arc<CrossSigningKeyPair> {
        CrossSigningKeyPair::wrap(CrossSigningKeyPair::duplicate(&self.inner.self_signing))
    }

    pub fn user_signing(&self) -> Arc<CrossSigningKeyPair> {
        CrossSigningKeyPair::wrap(CrossSigningKeyPair::duplicate(&self.inner.user_signing))
    }

    /// `POST /api/e2ee/cross_signing/keys` の本文（JSON）。
    pub fn upload_json(&self) -> String {
        self.inner.upload_json().to_string()
    }
}

/// サーバーから受け取ったクロス署名鍵の JSON を検証し、公開鍵（base64）を返す。署名は確かめない。
#[uniffi::export]
pub fn verify_cross_signing_key(json: String, user_id: String, usage: KeyUsage) -> Result<String> {
    let value = parse_json("json", &json)?;
    Ok(core::verify_cross_signing_key(&value, &user_id, usage.into())?.to_base64())
}

/// 公開鍵（Ed25519、base64）からクロス署名鍵の鍵 ID（`ed25519:<公開鍵>`）を作る。
#[uniffi::export]
pub fn cross_signing_key_id(public_key: String) -> Result<String> {
    Ok(core::cross_signing_key_id(&ed25519_from_base64(
        &public_key,
    )?))
}
