//! SSSS、鍵スロット、リカバリーキー（#351）。

use serde::Deserialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::backup::{BackupDecryptionKey, BackupKeyList};
use crate::convert::{IntoJs, bytes32, from_js, safe_u64_opt, to_js};
use crate::cross_signing::{CrossSigningKeyPair, CrossSigningKeys, parse_usage};

/// 新しい PRF の salt（32 バイト）。パスキーの鍵スロットを作るときに使う。
#[wasm_bindgen(js_name = newPrfSalt)]
pub fn new_prf_salt() -> Vec<u8> {
    core::new_prf_salt().to_vec()
}

/// リカバリーキー（32 バイト）。表示と入力は Base58 の文字列。
#[wasm_bindgen]
pub struct RecoveryKey {
    inner: core::RecoveryKey,
}

#[wasm_bindgen]
impl RecoveryKey {
    pub fn generate() -> RecoveryKey {
        Self {
            inner: core::RecoveryKey::generate(),
        }
    }

    /// 利用者が入力した文字列を読む（空白と大文字小文字の違いは無視する）。
    pub fn parse(input: &str) -> Result<RecoveryKey, JsValue> {
        Ok(Self {
            inner: core::RecoveryKey::parse(input).js()?,
        })
    }

    /// 表示する文字列。
    #[wasm_bindgen(js_name = toDisplay)]
    pub fn to_display(&self) -> String {
        self.inner.to_display().to_string()
    }
}

/// SSSS 鍵（32 バイト）と、その `key_id`。
#[wasm_bindgen]
pub struct SsssKey {
    pub(crate) inner: core::SsssKey,
}

#[derive(Deserialize)]
struct KeysRequestOptions {
    #[serde(default)]
    expected_default_key_id: Option<String>,
    #[serde(default)]
    expected_secrets_revision: Option<u64>,
}

#[derive(Deserialize)]
struct NamedSecret {
    name: String,
    secret: core::EncryptedSecret,
}

#[wasm_bindgen]
impl SsssKey {
    /// 新しい SSSS 鍵と `key_id`（UUID v4）を作る。
    pub fn generate() -> SsssKey {
        Self {
            inner: core::SsssKey::generate(),
        }
    }

    /// `keyId` と 32 バイトから作る。
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(key_id: &str, key: &[u8]) -> Result<SsssKey, JsValue> {
        let key = bytes32(key, "key")?;
        Ok(Self {
            inner: core::SsssKey::from_bytes(key_id, *key).js()?,
        })
    }

    #[wasm_bindgen(getter, js_name = keyId)]
    pub fn key_id(&self) -> String {
        self.inner.key_id().to_owned()
    }

    /// 鍵の 32 バイト。暗号ストアに保存するときだけ使い、使い終わったら消す。
    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }

    /// 鍵の説明 `{algorithm, iv, mac}`（確認値付き）。
    #[wasm_bindgen(unchecked_return_type = "SsssKeyDescription")]
    pub fn description(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.description())
    }

    /// 鍵が説明の確認値と一致するか確かめる。一致しなければ `SsssKeyMismatch` を投げる。
    pub fn check(
        &self,
        #[wasm_bindgen(unchecked_param_type = "SsssKeyDescription")] description: JsValue,
    ) -> Result<(), JsValue> {
        let description: core::SsssKeyDescription = from_js(&description, "description")?;
        self.inner.check(&description).js()
    }

    /// 秘密 `name` を暗号化する。
    #[wasm_bindgen(js_name = encryptSecret, unchecked_return_type = "EncryptedSecret")]
    pub fn encrypt_secret(&self, name: &str, plaintext: &str) -> Result<JsValue, JsValue> {
        to_js(&self.inner.encrypt_secret(name, plaintext))
    }

    /// 秘密 `name` を復号する。
    #[wasm_bindgen(js_name = decryptSecret)]
    pub fn decrypt_secret(
        &self,
        name: &str,
        #[wasm_bindgen(unchecked_param_type = "EncryptedSecret")] secret: JsValue,
    ) -> Result<String, JsValue> {
        let secret: core::EncryptedSecret = from_js(&secret, "secret")?;
        Ok(self.inner.decrypt_secret(name, &secret).js()?.to_string())
    }

    /// クロス署名鍵を暗号化し、`{name, secret}` を返す。
    #[wasm_bindgen(js_name = encryptCrossSigningKey, unchecked_return_type = "NamedSecret")]
    pub fn encrypt_cross_signing_key(&self, key: &CrossSigningKeyPair) -> Result<JsValue, JsValue> {
        let (name, secret) = self.inner.encrypt_cross_signing_key(&key.inner);
        to_js(&serde_json::json!({"name": name, "secret": secret}))
    }

    /// バックアップ鍵を暗号化する（`m.megolm_backup.v1`）。
    #[wasm_bindgen(js_name = encryptBackupKey, unchecked_return_type = "EncryptedSecret")]
    pub fn encrypt_backup_key(&self, key: &BackupDecryptionKey) -> Result<JsValue, JsValue> {
        to_js(&self.inner.encrypt_backup_key(&key.inner))
    }

    /// 古い版のバックアップ鍵を暗号化する（`zoo.megolm_backup.previous`）。
    #[wasm_bindgen(js_name = encryptPreviousBackupKeys, unchecked_return_type = "EncryptedSecret")]
    pub fn encrypt_previous_backup_keys(&self, keys: &BackupKeyList) -> Result<JsValue, JsValue> {
        to_js(&self.inner.encrypt_previous_backup_keys(&keys.inner))
    }

    /// `POST /api/e2ee/secret_storage/keys` の本文を作る。
    ///
    /// `options` は `{expected_default_key_id?, expected_secrets_revision?}`、`slots` は
    /// この鍵の鍵スロットの配列（`newPasskeySlot` か `newRecoverySlot` で作ったもの）。
    #[wasm_bindgen(js_name = keysRequest, unchecked_return_type = "JsonObject")]
    pub fn keys_request(
        &self,
        #[wasm_bindgen(unchecked_param_type = "KeysRequestOptions")] options: JsValue,
        cross_signing: &CrossSigningKeys,
        backup_key: &BackupDecryptionKey,
        previous_backup_keys: &BackupKeyList,
        #[wasm_bindgen(unchecked_param_type = "KeySlot[]")] slots: JsValue,
    ) -> Result<JsValue, JsValue> {
        let options: KeysRequestOptions = from_js(&options, "options")?;
        let slots: Vec<core::KeySlot> = from_js(&slots, "slots")?;
        let request = core::SecretStorageKeysRequest {
            expected_default_key_id: options.expected_default_key_id.as_deref(),
            expected_secrets_revision: options.expected_secrets_revision,
            cross_signing: &cross_signing.inner,
            backup_key: &backup_key.inner,
            previous_backup_keys: &previous_backup_keys.inner,
            slots: &slots,
        };
        to_js(&self.inner.keys_request(&request).js()?)
    }

    /// `POST /api/e2ee/secret_storage/secrets` の本文を作る。`secrets` は `{name, secret}` の配列。
    #[wasm_bindgen(js_name = secretsRequest, unchecked_return_type = "JsonObject")]
    pub fn secrets_request(
        &self,
        expected_secrets_revision: Option<f64>,
        #[wasm_bindgen(unchecked_param_type = "NamedSecret[]")] secrets: JsValue,
    ) -> Result<JsValue, JsValue> {
        let revision = safe_u64_opt(expected_secrets_revision, "expectedSecretsRevision")?;
        let secrets: Vec<NamedSecret> = from_js(&secrets, "secrets")?;
        let pairs: Vec<(&str, core::EncryptedSecret)> = secrets
            .iter()
            .map(|s| (s.name.as_str(), s.secret.clone()))
            .collect();
        to_js(&self.inner.secrets_request(revision, &pairs))
    }
}

/// パスキー（PRF）の鍵スロットを作る。`prfSalt` と `prfOutput` は 32 バイト。
#[wasm_bindgen(js_name = newPasskeySlot, unchecked_return_type = "KeySlot")]
pub fn new_passkey_slot(
    user_id: &str,
    ssss_key: &SsssKey,
    credential_id: &str,
    prf_salt: &[u8],
    prf_output: &[u8],
) -> Result<JsValue, JsValue> {
    let salt = bytes32(prf_salt, "prfSalt")?;
    let output = bytes32(prf_output, "prfOutput")?;
    to_js(
        &core::KeySlot::new_passkey(user_id, &ssss_key.inner, credential_id, &salt, &output)
            .js()?,
    )
}

/// リカバリーキーの鍵スロットを作る。
#[wasm_bindgen(js_name = newRecoverySlot, unchecked_return_type = "KeySlot")]
pub fn new_recovery_slot(
    user_id: &str,
    ssss_key: &SsssKey,
    recovery_key: &RecoveryKey,
) -> Result<JsValue, JsValue> {
    to_js(&core::KeySlot::new_recovery(user_id, &ssss_key.inner, &recovery_key.inner).js()?)
}

/// パスキーの鍵スロットの PRF の salt（32 バイト）。リカバリーキーのスロットなら `undefined`。
#[wasm_bindgen(js_name = keySlotPrfSalt)]
pub fn key_slot_prf_salt(
    #[wasm_bindgen(unchecked_param_type = "KeySlot")] slot: JsValue,
) -> Result<Option<Vec<u8>>, JsValue> {
    let slot: core::KeySlot = from_js(&slot, "slot")?;
    Ok(slot.prf_salt_bytes().js()?.map(|salt| salt.to_vec()))
}

/// パスキーの PRF 出力で鍵スロットを開ける。
#[wasm_bindgen(js_name = openKeySlotWithPasskey)]
pub fn open_key_slot_with_passkey(
    #[wasm_bindgen(unchecked_param_type = "KeySlot")] slot: JsValue,
    user_id: &str,
    credential_id: &str,
    prf_salt: &[u8],
    prf_output: &[u8],
) -> Result<SsssKey, JsValue> {
    let slot: core::KeySlot = from_js(&slot, "slot")?;
    let salt = bytes32(prf_salt, "prfSalt")?;
    let output = bytes32(prf_output, "prfOutput")?;
    let secret = core::SlotSecret::Passkey {
        credential_id,
        prf_salt: &salt,
        prf_output: &output,
    };
    Ok(SsssKey {
        inner: slot.open(user_id, secret).js()?,
    })
}

/// リカバリーキーで鍵スロットを開ける。
#[wasm_bindgen(js_name = openKeySlotWithRecoveryKey)]
pub fn open_key_slot_with_recovery_key(
    #[wasm_bindgen(unchecked_param_type = "KeySlot")] slot: JsValue,
    user_id: &str,
    recovery_key: &RecoveryKey,
) -> Result<SsssKey, JsValue> {
    let slot: core::KeySlot = from_js(&slot, "slot")?;
    Ok(SsssKey {
        inner: slot
            .open(user_id, core::SlotSecret::Recovery(&recovery_key.inner))
            .js()?,
    })
}

/// `GET /api/e2ee/secret_storage` の応答。解錠の入口。
#[wasm_bindgen]
pub struct SecretStorage {
    inner: core::SecretStorage,
}

#[wasm_bindgen]
impl SecretStorage {
    /// 応答を読む。SSSS がまだない（初回設定の前）なら `undefined` を返す。
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(
        user_id: &str,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] response: JsValue,
    ) -> Result<Option<SecretStorage>, JsValue> {
        let response: Value = from_js(&response, "response")?;
        Ok(core::SecretStorage::from_json(user_id, &response)
            .js()?
            .map(|inner| Self { inner }))
    }

    #[wasm_bindgen(getter, js_name = defaultKeyId)]
    pub fn default_key_id(&self) -> String {
        self.inner.default_key_id().to_owned()
    }

    #[wasm_bindgen(getter, js_name = secretsRevision)]
    pub fn secrets_revision(&self) -> Option<f64> {
        self.inner.secrets_revision().map(|r| r as f64)
    }

    /// 鍵スロットの一覧。
    #[wasm_bindgen(unchecked_return_type = "KeySlot[]")]
    pub fn slots(&self) -> Result<JsValue, JsValue> {
        to_js(self.inner.slots())
    }

    /// 今の SSSS 鍵のパスキーの鍵スロット（WebAuthn の `allowCredentials` と PRF の `evalByCredential` に使う）。
    #[wasm_bindgen(js_name = passkeySlots, unchecked_return_type = "KeySlot[]")]
    pub fn passkey_slots(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.passkey_slots().collect::<Vec<_>>())
    }

    /// パスキーの PRF 出力で解錠する。
    #[wasm_bindgen(js_name = unlockWithPasskey)]
    pub fn unlock_with_passkey(
        &self,
        credential_id: &str,
        prf_output: &[u8],
    ) -> Result<UnlockedSecretStorage, JsValue> {
        let output = bytes32(prf_output, "prfOutput")?;
        Ok(UnlockedSecretStorage {
            inner: self
                .inner
                .unlock_with_passkey(credential_id, &output)
                .js()?,
        })
    }

    /// リカバリーキーで解錠する。
    #[wasm_bindgen(js_name = unlockWithRecoveryKey)]
    pub fn unlock_with_recovery_key(
        &self,
        recovery_key: &RecoveryKey,
    ) -> Result<UnlockedSecretStorage, JsValue> {
        Ok(UnlockedSecretStorage {
            inner: self
                .inner
                .unlock_with_recovery_key(&recovery_key.inner)
                .js()?,
        })
    }

    /// メモリーにある SSSS 鍵で開ける（失効の始めと、保存のあとの読み直し。#362）。
    #[wasm_bindgen(js_name = unlockWithKey)]
    pub fn unlock_with_key(&self, key: &SsssKey) -> Result<UnlockedSecretStorage, JsValue> {
        Ok(UnlockedSecretStorage {
            inner: self.inner.unlock_with_key(&key.inner).js()?,
        })
    }
}

/// 解錠した SSSS。秘密を取り出す。
#[wasm_bindgen]
pub struct UnlockedSecretStorage {
    pub(crate) inner: core::UnlockedSecretStorage,
}

#[wasm_bindgen]
impl UnlockedSecretStorage {
    /// 解錠に使った SSSS 鍵。
    pub fn key(&self) -> SsssKey {
        SsssKey {
            inner: self.inner.key().clone(),
        }
    }

    #[wasm_bindgen(getter, js_name = secretsRevision)]
    pub fn secrets_revision(&self) -> Option<f64> {
        self.inner.secrets_revision().map(|r| r as f64)
    }

    /// 秘密 `name` を復号する。なければ `undefined`。
    pub fn secret(&self, name: &str) -> Result<Option<String>, JsValue> {
        Ok(self.inner.secret(name).js()?.map(|s| s.to_string()))
    }

    /// クロス署名鍵を取り出す。なければ `undefined`。
    #[wasm_bindgen(js_name = crossSigningKey)]
    pub fn cross_signing_key(
        &self,
        #[wasm_bindgen(unchecked_param_type = "KeyUsage")] usage: &str,
    ) -> Result<Option<CrossSigningKeyPair>, JsValue> {
        Ok(self
            .inner
            .cross_signing_key(parse_usage(usage)?)
            .js()?
            .map(|inner| CrossSigningKeyPair { inner }))
    }

    /// バックアップ鍵を取り出す。なければ `undefined`。
    #[wasm_bindgen(js_name = backupKey)]
    pub fn backup_key(&self) -> Result<Option<BackupDecryptionKey>, JsValue> {
        Ok(self
            .inner
            .backup_key()
            .js()?
            .map(|inner| BackupDecryptionKey { inner }))
    }

    /// 古い版のバックアップ鍵（`zoo.megolm_backup.previous`）を取り出す。
    #[wasm_bindgen(js_name = previousBackupKeys)]
    pub fn previous_backup_keys(&self) -> Result<BackupKeyList, JsValue> {
        Ok(BackupKeyList {
            inner: self.inner.previous_backup_keys().js()?,
        })
    }
}
