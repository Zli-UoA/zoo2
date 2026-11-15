//! SSSS、鍵スロット、リカバリーキー（#351）。
//!
//! サーバーとやり取りする形（鍵の説明、暗号化した秘密、鍵スロット）は JSON 文字列で渡す。

use std::sync::Arc;

use zoo_crypto as core;

use crate::{
    BackupDecryptionKey, CrossSigningKeyPair, CrossSigningKeys, KeyUsage, Result, ZooCryptoError,
    bytes32, parse_json,
};

fn from_json<T: serde::de::DeserializeOwned>(field: &str, json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| ZooCryptoError::InvalidPayload {
        reason: format!("{field} を読めません: {e}"),
    })
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("コアの型は必ず JSON にできる")
}

/// SSSS 鍵（[`core::SsssKey`]）。
#[derive(uniffi::Object)]
pub struct SsssKey {
    pub(crate) inner: core::SsssKey,
}

#[uniffi::export]
impl SsssKey {
    /// 新しい SSSS 鍵を作る（鍵 ID は新しい UUID v4）。
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Arc::new(Self {
            inner: core::SsssKey::generate(),
        })
    }

    #[uniffi::constructor]
    pub fn from_bytes(key_id: String, key: Vec<u8>) -> Result<Arc<Self>> {
        let key = zeroize::Zeroizing::new(bytes32("key", &key)?);
        Ok(Arc::new(Self {
            inner: core::SsssKey::from_bytes(&key_id, *key)?,
        }))
    }

    pub fn key_id(&self) -> String {
        self.inner.key_id().to_owned()
    }

    /// 鍵の 32 バイト（秘密）。
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    /// 鍵の説明（確認値、JSON）。
    pub fn description(&self) -> String {
        to_json(&self.inner.description())
    }

    /// 鍵の説明（JSON）の確認値と一致するかを確かめる。
    pub fn check(&self, description: String) -> Result<()> {
        Ok(self.inner.check(&from_json("description", &description)?)?)
    }

    /// 秘密を暗号化した JSON（`secrets.<name>`）。
    pub fn encrypt_secret(&self, name: String, plaintext: String) -> String {
        to_json(&self.inner.encrypt_secret(&name, &plaintext))
    }

    /// `secrets.<name>` の JSON を復号する。
    pub fn decrypt_secret(&self, name: String, secret: String) -> Result<String> {
        let secret = from_json("secret", &secret)?;
        Ok(self.inner.decrypt_secret(&name, &secret)?.to_string())
    }

    /// クロス署名鍵を暗号化する。
    pub fn encrypt_cross_signing_key(&self, key: &CrossSigningKeyPair) -> NamedSecret {
        let (name, secret) = self.inner.encrypt_cross_signing_key(&key.inner);
        NamedSecret {
            name: name.to_owned(),
            secret: to_json(&secret),
        }
    }

    /// バックアップ鍵を暗号化する（`m.megolm_backup.v1`）。
    pub fn encrypt_backup_key(&self, key: &BackupDecryptionKey) -> NamedSecret {
        NamedSecret {
            name: core::SECRET_MEGOLM_BACKUP.to_owned(),
            secret: to_json(&self.inner.encrypt_backup_key(&key.inner)),
        }
    }

    /// 古い版のバックアップ鍵を暗号化する（`zoo.megolm_backup.previous`）。
    pub fn encrypt_previous_backup_keys(&self, keys: Vec<Arc<BackupDecryptionKey>>) -> NamedSecret {
        let keys: Vec<core::BackupDecryptionKey> = keys.iter().map(|k| k.inner.clone()).collect();
        NamedSecret {
            name: core::SECRET_PREVIOUS_MEGOLM_BACKUP.to_owned(),
            secret: to_json(&self.inner.encrypt_previous_backup_keys(&keys)),
        }
    }

    /// `POST /api/e2ee/secret_storage/keys` の本文（JSON）。`slots` は鍵スロットの JSON の一覧。
    pub fn keys_request(
        &self,
        expected_default_key_id: Option<String>,
        expected_secrets_revision: Option<u64>,
        cross_signing: &CrossSigningKeys,
        backup_key: &BackupDecryptionKey,
        previous_backup_keys: Vec<Arc<BackupDecryptionKey>>,
        slots: Vec<String>,
    ) -> Result<String> {
        let previous: Vec<core::BackupDecryptionKey> = previous_backup_keys
            .iter()
            .map(|k| k.inner.clone())
            .collect();
        let slots = slots
            .iter()
            .map(|slot| from_json::<core::KeySlot>("slots", slot))
            .collect::<Result<Vec<_>>>()?;
        let request = core::SecretStorageKeysRequest {
            expected_default_key_id: expected_default_key_id.as_deref(),
            expected_secrets_revision,
            cross_signing: &cross_signing.inner,
            backup_key: &backup_key.inner,
            previous_backup_keys: &previous,
            slots: &slots,
        };
        Ok(self.inner.keys_request(&request)?.to_string())
    }

    /// `POST /api/e2ee/secret_storage/secrets` の本文（JSON）。
    pub fn secrets_request(
        &self,
        expected_secrets_revision: Option<u64>,
        secrets: Vec<NamedSecret>,
    ) -> Result<String> {
        let secrets = secrets
            .iter()
            .map(|s| Ok((s.name.as_str(), from_json("secrets", &s.secret)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(self
            .inner
            .secrets_request(expected_secrets_revision, &secrets)
            .to_string())
    }
}

/// 名前付きの暗号化した秘密。`secret` は `EncryptedSecret` の JSON。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NamedSecret {
    pub name: String,
    pub secret: String,
}

/// 新しいパスキーのスロットに使う 32 バイトの PRF の salt を作る。
#[uniffi::export]
pub fn new_prf_salt() -> Vec<u8> {
    core::new_prf_salt().to_vec()
}

/// パスキーの鍵スロット（JSON）を作る。
#[uniffi::export]
pub fn new_passkey_key_slot(
    user_id: String,
    ssss_key: &SsssKey,
    credential_id: String,
    prf_salt: Vec<u8>,
    prf_output: Vec<u8>,
) -> Result<String> {
    let prf_salt = bytes32("prf_salt", &prf_salt)?;
    let prf_output = zeroize::Zeroizing::new(bytes32("prf_output", &prf_output)?);
    let slot = core::KeySlot::new_passkey(
        &user_id,
        &ssss_key.inner,
        &credential_id,
        &prf_salt,
        &prf_output,
    )?;
    Ok(to_json(&slot))
}

/// リカバリーキーの鍵スロット（JSON）を作る。
#[uniffi::export]
pub fn new_recovery_key_slot(
    user_id: String,
    ssss_key: &SsssKey,
    recovery_key: &RecoveryKey,
) -> Result<String> {
    let slot = core::KeySlot::new_recovery(&user_id, &ssss_key.inner, &recovery_key.inner)?;
    Ok(to_json(&slot))
}

/// パスキーの鍵スロット 1 つ。`evalByCredential` に渡す credential ID と salt。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PasskeySlot {
    pub slot_id: String,
    pub credential_id: String,
    pub prf_salt: Vec<u8>,
    pub label: Option<String>,
}

/// `GET /api/e2ee/secret_storage` の応答（[`core::SecretStorage`]）。
#[derive(uniffi::Object)]
pub struct SecretStorage {
    inner: core::SecretStorage,
}

/// `user_id` の `GET /api/e2ee/secret_storage` の応答（JSON）を読む。SSSS がまだなければ `null`。
#[uniffi::export]
pub fn parse_secret_storage(
    user_id: String,
    response: String,
) -> Result<Option<Arc<SecretStorage>>> {
    let response = parse_json("response", &response)?;
    Ok(core::SecretStorage::from_json(&user_id, &response)?
        .map(|inner| Arc::new(SecretStorage { inner })))
}

#[uniffi::export]
impl SecretStorage {
    pub fn default_key_id(&self) -> String {
        self.inner.default_key_id().to_owned()
    }

    pub fn secrets_revision(&self) -> Option<u64> {
        self.inner.secrets_revision()
    }

    /// 鍵スロットの JSON の一覧。
    pub fn slots(&self) -> Vec<String> {
        self.inner.slots().iter().map(to_json).collect()
    }

    /// 既定の SSSS 鍵のパスキーのスロット。
    pub fn passkey_slots(&self) -> Result<Vec<PasskeySlot>> {
        self.inner
            .passkey_slots()
            .map(|slot| {
                Ok(PasskeySlot {
                    slot_id: slot.id.clone(),
                    credential_id: slot.credential_id.clone().unwrap_or_default(),
                    prf_salt: slot
                        .prf_salt_bytes()?
                        .map(|salt| salt.to_vec())
                        .unwrap_or_default(),
                    label: slot.label.clone(),
                })
            })
            .collect()
    }

    /// パスキーの PRF 出力で解錠する。
    pub fn unlock_with_passkey(
        &self,
        credential_id: String,
        prf_output: Vec<u8>,
    ) -> Result<Arc<UnlockedSecretStorage>> {
        let prf_output = zeroize::Zeroizing::new(bytes32("prf_output", &prf_output)?);
        Ok(Arc::new(UnlockedSecretStorage {
            inner: self
                .inner
                .unlock_with_passkey(&credential_id, &prf_output)?,
        }))
    }

    /// リカバリーキーで解錠する。
    pub fn unlock_with_recovery_key(
        &self,
        recovery_key: &RecoveryKey,
    ) -> Result<Arc<UnlockedSecretStorage>> {
        Ok(Arc::new(UnlockedSecretStorage {
            inner: self.inner.unlock_with_recovery_key(&recovery_key.inner)?,
        }))
    }
}

/// 解錠した SSSS（[`core::UnlockedSecretStorage`]）。
#[derive(uniffi::Object)]
pub struct UnlockedSecretStorage {
    pub(crate) inner: core::UnlockedSecretStorage,
}

#[uniffi::export]
impl UnlockedSecretStorage {
    pub fn key(&self) -> Arc<SsssKey> {
        Arc::new(SsssKey {
            inner: self.inner.key().clone(),
        })
    }

    pub fn secrets_revision(&self) -> Option<u64> {
        self.inner.secrets_revision()
    }

    /// 秘密 `name` を復号する。なければ `null`。
    pub fn secret(&self, name: String) -> Result<Option<String>> {
        Ok(self.inner.secret(&name)?.map(|s| s.to_string()))
    }

    pub fn cross_signing_key(&self, usage: KeyUsage) -> Result<Option<Arc<CrossSigningKeyPair>>> {
        Ok(self
            .inner
            .cross_signing_key(usage.into())?
            .map(CrossSigningKeyPair::wrap))
    }

    pub fn backup_key(&self) -> Result<Option<Arc<BackupDecryptionKey>>> {
        Ok(self.inner.backup_key()?.map(BackupDecryptionKey::wrap))
    }

    pub fn previous_backup_keys(&self) -> Result<Vec<Arc<BackupDecryptionKey>>> {
        Ok(self
            .inner
            .previous_backup_keys()?
            .into_iter()
            .map(BackupDecryptionKey::wrap)
            .collect())
    }
}

/// リカバリーキー（[`core::RecoveryKey`]）。
#[derive(uniffi::Object)]
pub struct RecoveryKey {
    inner: core::RecoveryKey,
}

#[uniffi::export]
impl RecoveryKey {
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Arc::new(Self {
            inner: core::RecoveryKey::generate(),
        })
    }

    /// 利用者が入力したリカバリーキーを読む（空白は無視する）。
    #[uniffi::constructor]
    pub fn parse(input: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: core::RecoveryKey::parse(&input)?,
        }))
    }

    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>> {
        let bytes = zeroize::Zeroizing::new(bytes32("bytes", &bytes)?);
        Ok(Arc::new(Self {
            inner: core::RecoveryKey::from_bytes(*bytes),
        }))
    }

    /// 利用者に見せる表記（Base58、4 文字ごとの区切り）。
    pub fn to_display(&self) -> String {
        self.inner.to_display().to_string()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }
}
