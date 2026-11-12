//! Secret Storage（SSSS）と鍵スロット（docs/e2ee.md の「SSSS（秘密の暗号化）」と「鍵スロット」）。
//!
//! - SSSS 鍵はランダムな 32 バイト。秘密は Matrix の `m.secret_storage.v1.aes-hmac-sha2` と同じ手順
//!   （HKDF-SHA256 で AES 鍵と MAC 鍵を導き、AES-256-CTR と HMAC-SHA256）で暗号化する。
//! - 鍵スロットは、パスキーの PRF 出力かリカバリーキーから HKDF-SHA256 で導いた KEK で、SSSS 鍵を
//!   AES-256-GCM で包んだもの。AAD に種類、ユーザー、SSSS 鍵、スロットの ID を長さ前置で入れるので、
//!   別のユーザー、別の SSSS 鍵、別のスロットに付け替えると開けなくなる。
//!
//! 端末は SSSS 鍵を保存しない。解錠のたびに鍵スロットを開け、確認値で確かめてから秘密を取り出す。

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use ctr::cipher::{KeyIvInit as _, StreamCipher as _};
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use vodozemac::{base64_decode, base64_encode};
use zeroize::{Zeroize, Zeroizing};

use crate::CryptoError;
use crate::backup::BackupDecryptionKey;
use crate::cross_signing::{CrossSigningKeyPair, CrossSigningKeys, KeyUsage};
use crate::device::{length_prefixed, new_uuid_v4, random_bytes, validate_uuid};
use crate::recovery_key::RecoveryKey;

/// SSSS 鍵の方式名。
pub const SSSS_ALGORITHM: &str = "m.secret_storage.v1.aes-hmac-sha2";
/// バックアップ鍵の秘密の名前。
pub const SECRET_MEGOLM_BACKUP: &str = "m.megolm_backup.v1";
/// 失効のあとに置く、古い版のバックアップ鍵の秘密の名前。
pub const SECRET_PREVIOUS_MEGOLM_BACKUP: &str = "zoo.megolm_backup.previous";
/// `POST /api/e2ee/secret_storage/keys` に必ず入れる 4 つの秘密の名前。
pub const REQUIRED_SECRETS: [&str; 4] = [
    "m.cross_signing.master",
    "m.cross_signing.self_signing",
    "m.cross_signing.user_signing",
    SECRET_MEGOLM_BACKUP,
];

/// パスキーのスロットの KEK を導く HKDF の info。
const PASSKEY_KEK_INFO: &[u8] = b"zoo e2ee ssss v1";
/// リカバリーキーのスロットの KEK を導く HKDF の info。
const RECOVERY_KEK_INFO: &[u8] = b"zoo e2ee ssss recovery v1";
/// 鍵スロットの AAD の先頭に置く文字列。
const SLOT_AAD_CONTEXT: &str = "zoo e2ee ssss slot v1";
/// 鍵スロットの暗号文（32 バイトの SSSS 鍵と 16 バイトのタグ）の長さ。
const SLOT_CIPHERTEXT_LEN: usize = 48;
/// WebAuthn の credential ID の最大長（WebAuthn Level 3 の 1023 バイト）。
const MAX_CREDENTIAL_ID_LEN: usize = 1023;

type HmacSha256 = Hmac<Sha256>;
type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

/// SSSS 鍵（ランダムな 32 バイト）と、その ID。
///
/// `Debug` は `key_id` だけを出す。落とすときにメモリーから消す。
#[derive(Clone)]
pub struct SsssKey {
    key_id: String,
    key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for SsssKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsssKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

/// SSSS 鍵の記述（`keys.<key_id>`）。確認値の `iv` と `mac` を持つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsssKeyDescription {
    /// 方式名。[`SSSS_ALGORITHM`] でなければならない。
    pub algorithm: String,
    /// 確認値の IV（16 バイト、パディングなしの base64）。
    pub iv: String,
    /// 確認値の MAC（32 バイト、パディングなしの base64）。
    pub mac: String,
}

/// SSSS で暗号化した秘密 1 つ（`secrets.<秘密の名前>`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
    /// 暗号化した SSSS 鍵の ID。`GET /api/e2ee/secret_storage` の応答にだけある。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// IV（16 バイト、パディングなしの base64）。
    pub iv: String,
    /// AES-256-CTR の暗号文（パディングなしの base64）。
    pub ciphertext: String,
    /// 暗号文の HMAC-SHA256（32 バイト、パディングなしの base64）。
    pub mac: String,
}

impl SsssKey {
    /// 新しい SSSS 鍵と `key_id`（UUID v4）を作る。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate() -> Self {
        Self {
            key_id: new_uuid_v4(),
            key: Zeroizing::new(random_bytes()),
        }
    }

    /// `key_id` と 32 バイトから作る（テストベクター用）。`key_id` は小文字でハイフン区切りの UUID。
    pub fn from_bytes(key_id: &str, key: [u8; 32]) -> Result<Self, CryptoError> {
        validate_uuid("key_id", key_id, false)?;
        Ok(Self {
            key_id: key_id.to_owned(),
            key: Zeroizing::new(key),
        })
    }

    /// SSSS 鍵の ID。
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// 32 バイトの中身（テストベクター用）。
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// SSSS 鍵の記述（確認値）を作る。32 バイトの 0 を、info を空文字列にして暗号化した
    /// ときの `iv` と `mac` を持つ。
    pub fn description(&self) -> SsssKeyDescription {
        self.description_with_iv(new_ctr_iv())
    }

    /// IV を指定して確認値を作る（テストベクター用）。
    #[doc(hidden)]
    pub fn description_with_iv(&self, iv: [u8; 16]) -> SsssKeyDescription {
        let (_, mac) = aes_hmac_encrypt(&self.key, "", &iv, &[0u8; 32]);
        SsssKeyDescription {
            algorithm: SSSS_ALGORITHM.to_owned(),
            iv: base64_encode(iv),
            mac: base64_encode(mac),
        }
    }

    /// この鍵が記述の確認値と合うかを確かめる。合わなければ [`CryptoError::SsssKeyMismatch`]。
    pub fn check(&self, description: &SsssKeyDescription) -> Result<(), CryptoError> {
        if description.algorithm != SSSS_ALGORITHM {
            return Err(CryptoError::InvalidSecretStorage(format!(
                "SSSS 鍵の方式が違います: {}",
                description.algorithm
            )));
        }
        let iv = decode_fixed::<16>("iv", &description.iv)?;
        let mac = decode_fixed::<32>("mac", &description.mac)?;
        let (ciphertext, _) = aes_hmac_encrypt(&self.key, "", &iv, &[0u8; 32]);
        verify_mac(&self.key, "", &ciphertext, &mac).map_err(|_| CryptoError::SsssKeyMismatch)
    }

    /// 秘密 `name` の平文（パディングなしの base64 など）を暗号化する。
    pub fn encrypt_secret(&self, name: &str, plaintext: &str) -> EncryptedSecret {
        self.encrypt_secret_with_iv(name, plaintext, new_ctr_iv())
    }

    /// IV を指定して秘密を暗号化する（テストベクター用）。
    #[doc(hidden)]
    pub fn encrypt_secret_with_iv(
        &self,
        name: &str,
        plaintext: &str,
        iv: [u8; 16],
    ) -> EncryptedSecret {
        let (ciphertext, mac) = aes_hmac_encrypt(&self.key, name, &iv, plaintext.as_bytes());
        EncryptedSecret {
            key_id: None,
            iv: base64_encode(iv),
            ciphertext: base64_encode(ciphertext),
            mac: base64_encode(mac),
        }
    }

    /// 秘密 `name` を復号する。
    ///
    /// `key_id` があってこの鍵と違う、または MAC が合わない（別の SSSS 鍵、別の名前、改ざん）ときは
    /// [`CryptoError::SecretDecryption`] を返す。
    pub fn decrypt_secret(
        &self,
        name: &str,
        secret: &EncryptedSecret,
    ) -> Result<Zeroizing<String>, CryptoError> {
        if secret.key_id.as_deref().is_some_and(|id| id != self.key_id) {
            return Err(CryptoError::SecretDecryption(format!(
                "{name} は別の SSSS 鍵で暗号化されています"
            )));
        }
        let iv = decode_fixed::<16>("iv", &secret.iv)?;
        let mac = decode_fixed::<32>("mac", &secret.mac)?;
        let ciphertext = base64_decode(&secret.ciphertext).map_err(|_| {
            CryptoError::InvalidSecretStorage("ciphertext が base64 ではありません".into())
        })?;
        verify_mac(&self.key, name, &ciphertext, &mac)
            .map_err(|_| CryptoError::SecretDecryption(format!("{name} の MAC が一致しません")))?;
        let mut plaintext = Zeroizing::new(ciphertext);
        let (aes_key, _) = derive_secret_keys(&self.key, name);
        Aes256Ctr::new_from_slices(aes_key.as_ref(), &iv)
            .expect("32 バイトの鍵と 16 バイトの IV")
            .apply_keystream(&mut plaintext);
        String::from_utf8(plaintext.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| {
                CryptoError::SecretDecryption(format!("{name} の平文が UTF-8 ではありません"))
            })
    }

    /// クロス署名鍵の秘密鍵（シードのパディングなしの base64）を暗号化する。名前は鍵の用途から決める。
    pub fn encrypt_cross_signing_key(
        &self,
        key: &CrossSigningKeyPair,
    ) -> (&'static str, EncryptedSecret) {
        let name = key.usage().secret_name();
        let plaintext = Zeroizing::new(base64_encode(key.seed().as_ref()));
        (name, self.encrypt_secret(name, &plaintext))
    }

    /// バックアップ鍵の秘密鍵を `m.megolm_backup.v1` として暗号化する。
    pub fn encrypt_backup_key(&self, key: &BackupDecryptionKey) -> EncryptedSecret {
        self.encrypt_secret(SECRET_MEGOLM_BACKUP, &key.to_base64())
    }

    /// 古い版のバックアップ鍵を `zoo.megolm_backup.previous` として暗号化する。
    pub fn encrypt_previous_backup_keys(&self, keys: &[BackupDecryptionKey]) -> EncryptedSecret {
        let plaintext = previous_backup_keys_json(keys);
        self.encrypt_secret(SECRET_PREVIOUS_MEGOLM_BACKUP, &plaintext)
    }

    /// `POST /api/e2ee/secret_storage/keys` の要求の本文を作る（初回設定、失効の 2〜5、リセット）。
    ///
    /// 4 つの秘密（クロス署名鍵 3 つとバックアップ鍵）を必ず入れ、`previous_backup_keys` が空でなければ
    /// `zoo.megolm_backup.previous` も入れる。`slots` が空、またはこの鍵のスロットでないものがあれば
    /// [`CryptoError::InvalidSecretStorage`] を返す（サーバーの 400 `E2EE_SLOTS_REQUIRED` と同じ考え方）。
    pub fn keys_request(
        &self,
        request: &SecretStorageKeysRequest<'_>,
    ) -> Result<Value, CryptoError> {
        if request.slots.is_empty() {
            return Err(CryptoError::InvalidSecretStorage(
                "鍵スロットが 1 つもありません".into(),
            ));
        }
        if let Some(slot) = request.slots.iter().find(|slot| slot.key_id != self.key_id) {
            return Err(CryptoError::InvalidSecretStorage(format!(
                "スロット {} は別の SSSS 鍵のものです",
                slot.id
            )));
        }
        let keys = request.cross_signing;
        for (key, usage) in [
            (&keys.master, KeyUsage::Master),
            (&keys.self_signing, KeyUsage::SelfSigning),
            (&keys.user_signing, KeyUsage::UserSigning),
        ] {
            if key.usage() != usage {
                return Err(CryptoError::InvalidCrossSigningKey(format!(
                    "{} の位置に {} の鍵があります",
                    usage.as_str(),
                    key.usage().as_str()
                )));
            }
        }
        let mut secrets = BTreeMap::new();
        for key in [&keys.master, &keys.self_signing, &keys.user_signing] {
            let (name, secret) = self.encrypt_cross_signing_key(key);
            secrets.insert(name, secret);
        }
        secrets.insert(
            SECRET_MEGOLM_BACKUP,
            self.encrypt_backup_key(request.backup_key),
        );
        if !request.previous_backup_keys.is_empty() {
            secrets.insert(
                SECRET_PREVIOUS_MEGOLM_BACKUP,
                self.encrypt_previous_backup_keys(request.previous_backup_keys),
            );
        }
        Ok(json!({
            "expected_default_key_id": request.expected_default_key_id,
            "expected_secrets_revision": request.expected_secrets_revision,
            "key_id": self.key_id,
            "key": self.description(),
            "secrets": secrets,
            "slots": request.slots,
        }))
    }

    /// `POST /api/e2ee/secret_storage/secrets` の要求の本文を作る（端末だけの失効など）。
    ///
    /// `secrets` は秘密の名前と、この鍵で暗号化した秘密の組。
    pub fn secrets_request(
        &self,
        expected_secrets_revision: Option<u64>,
        secrets: &[(&str, EncryptedSecret)],
    ) -> Value {
        let secrets: BTreeMap<&str, &EncryptedSecret> = secrets
            .iter()
            .map(|(name, secret)| (*name, secret))
            .collect();
        json!({
            "key_id": self.key_id,
            "expected_secrets_revision": expected_secrets_revision,
            "secrets": secrets,
        })
    }
}

/// [`SsssKey::keys_request`] に渡すもの。
#[derive(Debug)]
pub struct SecretStorageKeysRequest<'a> {
    /// 今の既定の SSSS 鍵の ID（初回設定では `None`）。
    pub expected_default_key_id: Option<&'a str>,
    /// 今の `secrets_revision`（初回設定では `None`）。
    pub expected_secrets_revision: Option<u64>,
    /// SSSS に入れるクロス署名鍵。
    pub cross_signing: &'a CrossSigningKeys,
    /// SSSS に入れるバックアップ鍵。
    pub backup_key: &'a BackupDecryptionKey,
    /// `zoo.megolm_backup.previous` に入れる古い版のバックアップ鍵（なければ空）。
    pub previous_backup_keys: &'a [BackupDecryptionKey],
    /// この SSSS 鍵の鍵スロット（1 つ以上）。
    pub slots: &'a [KeySlot],
}

/// 鍵スロットの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    /// パスキーの PRF 出力で開ける。
    Passkey,
    /// リカバリーキーで開ける。
    Recovery,
}

impl SlotKind {
    /// AAD と JSON に入れる名前（`passkey` か `recovery`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passkey => "passkey",
            Self::Recovery => "recovery",
        }
    }
}

/// 鍵スロット 1 つ（`GET /api/e2ee/secret_storage` の `slots` の要素と、作るときの要求の形）。
///
/// サーバーが付ける `created_at` や `last_used_at` などは読まずに捨てる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySlot {
    /// スロットの ID（クライアントが作る UUID）。
    pub id: String,
    /// 包んでいる SSSS 鍵の ID。
    pub key_id: String,
    /// 種類。
    pub kind: SlotKind,
    /// パスキーの credential ID（base64url、パディングなし）。パスキーのスロットだけにある。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// PRF の salt（32 バイト、パディングなしの base64）。パスキーのスロットだけにある。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prf_salt: Option<String>,
    /// AES-256-GCM の nonce（12 バイト、パディングなしの base64）。
    pub nonce: String,
    /// SSSS 鍵の暗号文とタグ（48 バイト、パディングなしの base64）。
    pub ciphertext: String,
    /// 利用者が付けた名前（任意）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// 新しいパスキーのスロットに使う、32 バイトの PRF の salt を作る。
///
/// スロットを作るたびに新しくする（作り直すときも必ず新しくする）。`navigator.credentials.get` の
/// `extensions.prf.eval.first` に渡し、返った PRF 出力と一緒に [`KeySlot::new_passkey`] に渡す。
///
/// # Panics
///
/// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
pub fn new_prf_salt() -> [u8; 32] {
    random_bytes()
}

impl KeySlot {
    /// パスキーのスロットを作る。
    ///
    /// `prf_salt` は [`new_prf_salt`] で作って PRF の評価に渡したもの、`prf_output` はその PRF 出力の
    /// `first`（32 バイト）。`slot_id` は新しい UUID v4 にする。
    pub fn new_passkey(
        user_id: &str,
        ssss_key: &SsssKey,
        credential_id: &str,
        prf_salt: &[u8; 32],
        prf_output: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        Self::seal(
            user_id,
            ssss_key,
            &new_uuid_v4(),
            SlotSecret::Passkey {
                credential_id,
                prf_salt,
                prf_output,
            },
            random_bytes(),
        )
    }

    /// リカバリーキーのスロットを作る。`slot_id` は新しい UUID v4 にする。
    pub fn new_recovery(
        user_id: &str,
        ssss_key: &SsssKey,
        recovery_key: &RecoveryKey,
    ) -> Result<Self, CryptoError> {
        Self::seal(
            user_id,
            ssss_key,
            &new_uuid_v4(),
            SlotSecret::Recovery(recovery_key),
            random_bytes(),
        )
    }

    /// `slot_id` と `nonce` を指定してスロットを作る（テストベクター用）。
    #[doc(hidden)]
    pub fn seal(
        user_id: &str,
        ssss_key: &SsssKey,
        slot_id: &str,
        secret: SlotSecret<'_>,
        nonce: [u8; 12],
    ) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        validate_uuid("slot_id", slot_id, false)?;
        let (kind, credential_id, prf_salt) = match &secret {
            SlotSecret::Passkey {
                credential_id,
                prf_salt,
                ..
            } => {
                validate_credential_id(credential_id)?;
                (
                    SlotKind::Passkey,
                    Some((*credential_id).to_owned()),
                    Some(base64_encode(prf_salt)),
                )
            }
            SlotSecret::Recovery(_) => (SlotKind::Recovery, None, None),
        };
        let kek = secret.kek();
        let aad = slot_aad(kind, user_id, ssss_key.key_id(), slot_id);
        let ciphertext = Aes256Gcm::new_from_slice(kek.as_ref())
            .expect("32 バイトの鍵")
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: ssss_key.as_bytes(),
                    aad: &aad,
                },
            )
            .expect("AES-GCM の暗号化は長さの上限を超えない限り失敗しない");
        Ok(Self {
            id: slot_id.to_owned(),
            key_id: ssss_key.key_id().to_owned(),
            kind,
            credential_id,
            prf_salt,
            nonce: base64_encode(nonce),
            ciphertext: base64_encode(ciphertext),
            label: None,
        })
    }

    /// PRF の salt（32 バイト）。パスキーのスロットだけにある。`evalByCredential` に渡す。
    pub fn prf_salt_bytes(&self) -> Result<Option<[u8; 32]>, CryptoError> {
        self.prf_salt
            .as_deref()
            .map(|salt| decode_fixed::<32>("prf_salt", salt))
            .transpose()
    }

    /// `user_id` のスロットとして開け、SSSS 鍵を取り出す。確認値は確かめないので、呼び出し側は
    /// [`SsssKey::check`] で確かめる（[`SecretStorage`] の解錠は両方を行う）。
    ///
    /// - スロットの形式が壊れている（ID が UUID でない、種類と項目が合わない、長さが違う）ときは
    ///   [`CryptoError::InvalidSecretStorage`]。
    /// - `secret` の種類がスロットと違うときと、復号できない（PRF 出力やリカバリーキーが違う、
    ///   スロットが改ざんされた、別のユーザー、SSSS 鍵、スロットに付け替えられた）ときは
    ///   [`CryptoError::KeySlotUnlock`]。
    pub fn open(&self, user_id: &str, secret: SlotSecret<'_>) -> Result<SsssKey, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        self.validate()?;
        let kind_matches = match &secret {
            SlotSecret::Passkey {
                credential_id,
                prf_salt,
                ..
            } => {
                self.kind == SlotKind::Passkey
                    && self.credential_id.as_deref() == Some(*credential_id)
                    && self.prf_salt_bytes()?.as_ref() == Some(*prf_salt)
            }
            SlotSecret::Recovery(_) => self.kind == SlotKind::Recovery,
        };
        if !kind_matches {
            return Err(CryptoError::KeySlotUnlock);
        }
        let nonce = decode_fixed::<12>("nonce", &self.nonce)?;
        let ciphertext = decode_fixed::<SLOT_CIPHERTEXT_LEN>("ciphertext", &self.ciphertext)?;
        let kek = secret.kek();
        let aad = slot_aad(self.kind, user_id, &self.key_id, &self.id);
        let plaintext = Zeroizing::new(
            Aes256Gcm::new_from_slice(kek.as_ref())
                .expect("32 バイトの鍵")
                .decrypt(
                    &Nonce::from(nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| CryptoError::KeySlotUnlock)?,
        );
        let mut key = [0u8; 32];
        key.copy_from_slice(&plaintext);
        let ssss_key = SsssKey::from_bytes(&self.key_id, key);
        key.zeroize();
        ssss_key
    }

    /// サーバーから受け取ったスロットの形式を確かめる。
    fn validate(&self) -> Result<(), CryptoError> {
        let invalid = |e: CryptoError| CryptoError::InvalidSecretStorage(e.to_string());
        validate_uuid("slot_id", &self.id, false).map_err(invalid)?;
        validate_uuid("key_id", &self.key_id, false).map_err(invalid)?;
        match (self.kind, &self.credential_id, &self.prf_salt) {
            (SlotKind::Passkey, Some(credential_id), Some(_)) => {
                validate_credential_id(credential_id).map_err(invalid)?;
                self.prf_salt_bytes()?;
            }
            (SlotKind::Recovery, None, None) => {}
            _ => {
                return Err(CryptoError::InvalidSecretStorage(format!(
                    "{} のスロットの credential_id と prf_salt が正しくありません",
                    self.kind.as_str()
                )));
            }
        }
        Ok(())
    }
}

/// 鍵スロットを開ける手段。
pub enum SlotSecret<'a> {
    /// パスキーの PRF 出力。`credential_id` と `prf_salt` は、どのスロットの評価かを照合するのに使う。
    Passkey {
        /// パスキーの credential ID（base64url、パディングなし）。
        credential_id: &'a str,
        /// PRF の評価に渡した salt。
        prf_salt: &'a [u8; 32],
        /// PRF 出力の `first`。
        prf_output: &'a [u8; 32],
    },
    /// リカバリーキー。
    Recovery(&'a RecoveryKey),
}

impl std::fmt::Debug for SlotSecret<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passkey { credential_id, .. } => f
                .debug_struct("Passkey")
                .field("credential_id", credential_id)
                .finish_non_exhaustive(),
            Self::Recovery(_) => f.write_str("Recovery(..)"),
        }
    }
}

impl SlotSecret<'_> {
    /// KEK を導く。`HKDF-SHA256(ikm, salt = なし, info, L = 32)`。
    fn kek(&self) -> Zeroizing<[u8; 32]> {
        let (ikm, info): (&[u8], &[u8]) = match self {
            Self::Passkey { prf_output, .. } => (prf_output.as_slice(), PASSKEY_KEK_INFO),
            Self::Recovery(key) => (key.as_bytes().as_slice(), RECOVERY_KEK_INFO),
        };
        let mut kek = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, ikm)
            .expand(info, kek.as_mut_slice())
            .expect("32 バイトは HKDF-SHA256 の上限より短い");
        kek
    }
}

/// `AAD = LP("zoo e2ee ssss slot v1") ‖ LP(kind) ‖ LP(user_id) ‖ LP(key_id) ‖ LP(slot_id)`。
fn slot_aad(kind: SlotKind, user_id: &str, key_id: &str, slot_id: &str) -> Vec<u8> {
    length_prefixed(&[SLOT_AAD_CONTEXT, kind.as_str(), user_id, key_id, slot_id])
}

/// `GET /api/e2ee/secret_storage` の応答。解錠の入口。
#[derive(Debug, Clone)]
pub struct SecretStorage {
    user_id: String,
    default_key_id: String,
    description: SsssKeyDescription,
    secrets: BTreeMap<String, EncryptedSecret>,
    secrets_revision: Option<u64>,
    slots: Vec<KeySlot>,
}

#[derive(Deserialize)]
struct SecretStorageResponse {
    default_key_id: Option<String>,
    #[serde(default)]
    keys: BTreeMap<String, SsssKeyDescription>,
    #[serde(default)]
    secrets: BTreeMap<String, EncryptedSecret>,
    #[serde(default)]
    secrets_revision: Option<u64>,
    #[serde(default)]
    slots: Vec<KeySlot>,
}

impl SecretStorage {
    /// `user_id` の `GET /api/e2ee/secret_storage` の応答を読む。
    ///
    /// 既定の鍵がない（初回設定がまだ）ときは `Ok(None)` を返す。既定の鍵の記述がない、ID が UUID でない
    /// などの形式の誤りは [`CryptoError::InvalidSecretStorage`]。既定の鍵でないスロットは捨てる。
    pub fn from_json(user_id: &str, response: &Value) -> Result<Option<Self>, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        let response: SecretStorageResponse = serde_json::from_value(response.clone())
            .map_err(|e| CryptoError::InvalidSecretStorage(format!("応答の形式が違います: {e}")))?;
        let Some(default_key_id) = response.default_key_id else {
            return Ok(None);
        };
        validate_uuid("key_id", &default_key_id, false)
            .map_err(|e| CryptoError::InvalidSecretStorage(e.to_string()))?;
        let description = response.keys.get(&default_key_id).cloned().ok_or_else(|| {
            CryptoError::InvalidSecretStorage("既定の SSSS 鍵の記述がありません".into())
        })?;
        let slots = response
            .slots
            .into_iter()
            .filter(|slot| slot.key_id == default_key_id)
            .collect();
        Ok(Some(Self {
            user_id: user_id.to_owned(),
            default_key_id,
            description,
            secrets: response.secrets,
            secrets_revision: response.secrets_revision,
            slots,
        }))
    }

    /// 既定の SSSS 鍵の ID。
    pub fn default_key_id(&self) -> &str {
        &self.default_key_id
    }

    /// `secrets_revision`。
    pub fn secrets_revision(&self) -> Option<u64> {
        self.secrets_revision
    }

    /// 既定の鍵のスロット。
    pub fn slots(&self) -> &[KeySlot] {
        &self.slots
    }

    /// 既定の鍵のパスキーのスロット（`allowCredentials` と `evalByCredential` に使う）。
    pub fn passkey_slots(&self) -> impl Iterator<Item = &KeySlot> {
        self.slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Passkey)
    }

    /// パスキーで解錠する。`credential_id` は返った `credential.rawId` の base64url、`prf_output` は
    /// PRF 出力の `first`。
    ///
    /// その credential ID のスロットがなければ [`CryptoError::NoKeySlot`]、開けなければ
    /// [`CryptoError::KeySlotUnlock`]、確認値が合わなければ [`CryptoError::SsssKeyMismatch`]。
    pub fn unlock_with_passkey(
        &self,
        credential_id: &str,
        prf_output: &[u8; 32],
    ) -> Result<UnlockedSecretStorage, CryptoError> {
        let slot = self
            .passkey_slots()
            .find(|slot| slot.credential_id.as_deref() == Some(credential_id))
            .ok_or(CryptoError::NoKeySlot)?;
        let prf_salt = slot.prf_salt_bytes()?.ok_or_else(|| {
            CryptoError::InvalidSecretStorage("パスキーのスロットに prf_salt がありません".into())
        })?;
        self.unlock(
            slot,
            SlotSecret::Passkey {
                credential_id,
                prf_salt: &prf_salt,
                prf_output,
            },
        )
    }

    /// リカバリーキーで解錠する。リカバリーキーのスロットがなければ [`CryptoError::NoKeySlot`]。
    /// ほかは [`unlock_with_passkey`](Self::unlock_with_passkey) と同じ。
    pub fn unlock_with_recovery_key(
        &self,
        recovery_key: &RecoveryKey,
    ) -> Result<UnlockedSecretStorage, CryptoError> {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.kind == SlotKind::Recovery)
            .ok_or(CryptoError::NoKeySlot)?;
        self.unlock(slot, SlotSecret::Recovery(recovery_key))
    }

    fn unlock(
        &self,
        slot: &KeySlot,
        secret: SlotSecret<'_>,
    ) -> Result<UnlockedSecretStorage, CryptoError> {
        let key = slot.open(&self.user_id, secret)?;
        key.check(&self.description)?;
        Ok(UnlockedSecretStorage {
            user_id: self.user_id.clone(),
            key,
            secrets: self.secrets.clone(),
            secrets_revision: self.secrets_revision,
        })
    }
}

/// 解錠した SSSS。確認値で確かめた SSSS 鍵と、暗号化したままの秘密を持つ。
///
/// 端末はこれを保存しない（SSSS 鍵は保存しない）。
#[derive(Debug, Clone)]
pub struct UnlockedSecretStorage {
    user_id: String,
    key: SsssKey,
    secrets: BTreeMap<String, EncryptedSecret>,
    secrets_revision: Option<u64>,
}

impl UnlockedSecretStorage {
    /// 確認値で確かめた SSSS 鍵。
    pub fn key(&self) -> &SsssKey {
        &self.key
    }

    /// 解錠したときの `secrets_revision`。暗号ストアに記録する。
    pub fn secrets_revision(&self) -> Option<u64> {
        self.secrets_revision
    }

    /// 秘密 `name` を復号する。なければ `Ok(None)`。
    pub fn secret(&self, name: &str) -> Result<Option<Zeroizing<String>>, CryptoError> {
        self.secrets
            .get(name)
            .map(|secret| self.key.decrypt_secret(name, secret))
            .transpose()
    }

    /// クロス署名鍵の秘密鍵を取り出す。なければ `Ok(None)`。公開鍵がサーバーの鍵と一致するかは、
    /// 呼び出し側が確かめる（docs/e2ee.md の「新しい端末での解錠」の 3）。
    pub fn cross_signing_key(
        &self,
        usage: KeyUsage,
    ) -> Result<Option<CrossSigningKeyPair>, CryptoError> {
        let Some(plaintext) = self.secret(usage.secret_name())? else {
            return Ok(None);
        };
        let seed = decode_secret_bytes(usage.secret_name(), &plaintext)?;
        CrossSigningKeyPair::from_seed(&self.user_id, usage, &seed).map(Some)
    }

    /// バックアップ鍵の秘密鍵（`m.megolm_backup.v1`）を取り出す。なければ `Ok(None)`。
    pub fn backup_key(&self) -> Result<Option<BackupDecryptionKey>, CryptoError> {
        let Some(plaintext) = self.secret(SECRET_MEGOLM_BACKUP)? else {
            return Ok(None);
        };
        let bytes = decode_secret_bytes(SECRET_MEGOLM_BACKUP, &plaintext)?;
        Ok(Some(BackupDecryptionKey::from_bytes(&bytes)))
    }

    /// 古い版のバックアップ鍵（`zoo.megolm_backup.previous`）を取り出す。なければ空。
    ///
    /// 各要素の秘密鍵から導いた公開鍵が `public_key` と一致しなければ
    /// [`CryptoError::InvalidSecretStorage`] を返す。
    pub fn previous_backup_keys(&self) -> Result<Vec<BackupDecryptionKey>, CryptoError> {
        let Some(plaintext) = self.secret(SECRET_PREVIOUS_MEGOLM_BACKUP)? else {
            return Ok(Vec::new());
        };
        parse_previous_backup_keys(&plaintext)
    }
}

/// `zoo.megolm_backup.previous` の平文 `{"keys": [{"public_key", "private_key"}]}` を作る。
fn previous_backup_keys_json(keys: &[BackupDecryptionKey]) -> Zeroizing<String> {
    let keys: Vec<Value> = keys
        .iter()
        .map(|key| {
            json!({
                "public_key": key.public_key().to_base64(),
                "private_key": *key.to_base64(),
            })
        })
        .collect();
    let mut value = json!({ "keys": keys });
    let text = Zeroizing::new(value.to_string());
    zeroize_json(&mut value);
    text
}

/// `zoo.megolm_backup.previous` の平文を読み、秘密鍵と公開鍵の対応を確かめる。
fn parse_previous_backup_keys(plaintext: &str) -> Result<Vec<BackupDecryptionKey>, CryptoError> {
    #[derive(Deserialize)]
    struct Previous {
        keys: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        public_key: String,
        private_key: String,
    }
    let invalid = |message: &str| {
        CryptoError::InvalidSecretStorage(format!("{SECRET_PREVIOUS_MEGOLM_BACKUP} {message}"))
    };
    let previous: Previous =
        serde_json::from_str(plaintext).map_err(|_| invalid("の形式が違います"))?;
    previous
        .keys
        .into_iter()
        .map(|entry| {
            let bytes = decode_secret_bytes(SECRET_PREVIOUS_MEGOLM_BACKUP, &entry.private_key)?;
            let key = BackupDecryptionKey::from_bytes(&bytes);
            let public_key = vodozemac::Curve25519PublicKey::from_base64(&entry.public_key)
                .map_err(|_| invalid("の public_key が鍵ではありません"))?;
            if key.public_key() != public_key {
                return Err(invalid("の秘密鍵と公開鍵が対応していません"));
            }
            Ok(key)
        })
        .collect()
}

/// JSON の中の文字列を消す（秘密鍵を含む JSON を落とす前に使う）。
fn zeroize_json(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(zeroize_json),
        Value::Object(map) => map.values_mut().for_each(zeroize_json),
        _ => {}
    }
}

/// 秘密の平文（32 バイトのパディングなしの base64）を読む。
fn decode_secret_bytes(name: &str, plaintext: &str) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let bytes = Zeroizing::new(base64_decode(plaintext).map_err(|_| {
        CryptoError::InvalidSecretStorage(format!("{name} が base64 ではありません"))
    })?);
    let mut out = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return Err(CryptoError::InvalidSecretStorage(format!(
            "{name} が 32 バイトではありません"
        )));
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// 長さの決まったバイト列をパディングなしの base64 から読む。
pub(crate) fn decode_fixed<const N: usize>(
    field: &str,
    value: &str,
) -> Result<[u8; N], CryptoError> {
    base64_decode(value)
        .ok()
        .and_then(|bytes| <[u8; N]>::try_from(bytes).ok())
        .ok_or_else(|| {
            CryptoError::InvalidSecretStorage(format!(
                "{field} が {N} バイトの base64 ではありません"
            ))
        })
}

/// credential ID が base64url（パディングなし）で、1〜1023 バイトかを確かめる。
fn validate_credential_id(credential_id: &str) -> Result<(), CryptoError> {
    let valid_chars = credential_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let len = credential_id.len();
    let decoded_len = len / 4 * 3 + (len % 4).saturating_sub(1);
    if valid_chars && len % 4 != 1 && (1..=MAX_CREDENTIAL_ID_LEN).contains(&decoded_len) {
        Ok(())
    } else {
        Err(CryptoError::InvalidId {
            field: "credential_id",
            value: credential_id.to_owned(),
        })
    }
}

/// 秘密の暗号化に使う 16 バイトの IV。bit 63 を 0 にする（AES-CTR の実装の差を避けるため）。
fn new_ctr_iv() -> [u8; 16] {
    let mut iv: [u8; 16] = random_bytes();
    iv[8] &= 0x7f;
    iv
}

/// `HKDF-SHA256(ikm = SSSS 鍵, salt = 32 バイトの 0, info = 秘密の名前)` の 64 バイトを、AES 鍵と MAC 鍵にする。
fn derive_secret_keys(key: &[u8; 32], name: &str) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let mut okm = Zeroizing::new([0u8; 64]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), key)
        .expand(name.as_bytes(), okm.as_mut_slice())
        .expect("64 バイトは HKDF-SHA256 の上限より短い");
    let mut aes_key = Zeroizing::new([0u8; 32]);
    let mut mac_key = Zeroizing::new([0u8; 32]);
    aes_key.copy_from_slice(&okm[..32]);
    mac_key.copy_from_slice(&okm[32..]);
    (aes_key, mac_key)
}

/// AES-256-CTR で暗号化し、暗号文の HMAC-SHA256 を付ける。
fn aes_hmac_encrypt(
    key: &[u8; 32],
    name: &str,
    iv: &[u8; 16],
    plaintext: &[u8],
) -> (Vec<u8>, [u8; 32]) {
    let (aes_key, mac_key) = derive_secret_keys(key, name);
    let mut ciphertext = plaintext.to_vec();
    Aes256Ctr::new_from_slices(aes_key.as_ref(), iv)
        .expect("32 バイトの鍵と 16 バイトの IV")
        .apply_keystream(&mut ciphertext);
    let mut mac = <HmacSha256 as hmac::KeyInit>::new_from_slice(mac_key.as_ref())
        .expect("HMAC はどの長さの鍵も受け付ける");
    mac.update(&ciphertext);
    (ciphertext, mac.finalize().into_bytes().into())
}

/// 暗号文の HMAC-SHA256 を定数時間で比べる。
fn verify_mac(key: &[u8; 32], name: &str, ciphertext: &[u8], mac: &[u8; 32]) -> Result<(), ()> {
    let (_, mac_key) = derive_secret_keys(key, name);
    let mut hmac = <HmacSha256 as hmac::KeyInit>::new_from_slice(mac_key.as_ref())
        .expect("HMAC はどの長さの鍵も受け付ける");
    hmac.update(ciphertext);
    hmac.verify_slice(mac).map_err(|_| ())
}
