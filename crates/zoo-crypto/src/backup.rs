//! 鍵バックアップ（docs/e2ee.md の「鍵バックアップ」）。
//!
//! 方式は `zoo.megolm_backup.v1.curve25519-aes-sha2`。Matrix の `m.megolm_backup.v1.curve25519-aes-sha2` と
//! 同じ手順で、HKDF の info を `"zoo e2ee megolm backup v1"` にし、MAC を AES-256-CBC の暗号文の
//! HMAC-SHA256 の 32 バイトすべてにしたもの。vodozemac の `insecure-pk-encryption` は使わない。
//!
//! - [`BackupDecryptionKey`]：バックアップ鍵の秘密鍵（SSSS の `m.megolm_backup.v1`）
//! - [`BackupEncryptionKey`]：今の版の公開鍵。セッションを暗号化する
//! - [`verify_backup_version`]：今の版の検証（マスター鍵の署名と、SSSS の鍵との一致）
//! - [`BackupUploadState`]：アップロード待ちの記録と、今の版の検証の状態機械
//! - [`select_previous_backup_keys`]、[`previous_versions_to_migrate`]：古い版の秘密鍵の退避と移行

use std::collections::{BTreeMap, BTreeSet};

use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt as _, BlockModeEncrypt as _, KeyIvInit as _};
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use vodozemac::{
    Curve25519PublicKey, Curve25519SecretKey, Ed25519PublicKey, base64_decode, base64_encode,
};
use zeroize::{Zeroize, Zeroizing};

use crate::CryptoError;
use crate::cross_signing::{CrossSigningKeyPair, cross_signing_key_id};
use crate::device::{MEGOLM_ALGORITHM, random_bytes, validate_uuid};
use crate::inbound::{BackupSession, InboundSessionStore, SessionRef};
use crate::signing::verify_json_signature;

/// 鍵バックアップの方式名。
pub const BACKUP_ALGORITHM: &str = "zoo.megolm_backup.v1.curve25519-aes-sha2";
/// HKDF の info。
const BACKUP_INFO: &[u8] = b"zoo e2ee megolm backup v1";
/// `POST /api/e2ee/key_backup/keys` の 1 回の上限。
pub const MAX_UPLOAD_SESSIONS: usize = 1000;

type HmacSha256 = Hmac<Sha256>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// バックアップ鍵の秘密鍵（Curve25519）。SSSS の `m.megolm_backup.v1` に置く。
///
/// `Debug` は公開鍵だけを出す。
pub struct BackupDecryptionKey {
    secret: Curve25519SecretKey,
    public: Curve25519PublicKey,
}

impl std::fmt::Debug for BackupDecryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupDecryptionKey")
            .field("public_key", &self.public)
            .finish_non_exhaustive()
    }
}

impl Clone for BackupDecryptionKey {
    fn clone(&self) -> Self {
        Self::from_bytes(&self.to_bytes())
    }
}

impl BackupDecryptionKey {
    /// 新しいバックアップ鍵を作る。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate() -> Self {
        let mut bytes: [u8; 32] = random_bytes();
        let key = Self::from_bytes(&bytes);
        bytes.zeroize();
        key
    }

    /// 32 バイトの秘密鍵から作る。
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let secret = Curve25519SecretKey::from_slice(bytes);
        let public = Curve25519PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 32 バイトの秘密鍵。
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        let mut bytes = self.secret.to_bytes();
        let out = Zeroizing::new(*bytes);
        bytes.zeroize();
        out
    }

    /// SSSS に置く平文（秘密鍵のパディングなしの base64）。
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(base64_encode(self.to_bytes().as_ref()))
    }

    /// 公開鍵（版の `auth_data.public_key`）。
    pub fn public_key(&self) -> Curve25519PublicKey {
        self.public
    }

    /// 暗号化に使う公開鍵。
    pub fn encryption_key(&self) -> BackupEncryptionKey {
        BackupEncryptionKey(self.public)
    }

    /// 版の `auth_data`（`{public_key, signatures}`）を作り、マスター鍵で署名する。
    pub fn auth_data(&self, master: &CrossSigningKeyPair) -> Result<Value, CryptoError> {
        let mut auth_data = json!({ "public_key": self.public.to_base64() });
        master.sign_auth_data(&mut auth_data)?;
        Ok(auth_data)
    }

    /// `POST /api/e2ee/key_backup/versions` の要求の本文を作る。
    ///
    /// `expected_current_version` は今の版（なければ `None`）。
    pub fn create_version_request(
        &self,
        master: &CrossSigningKeyPair,
        expected_current_version: Option<&str>,
    ) -> Result<Value, CryptoError> {
        Ok(json!({
            "algorithm": BACKUP_ALGORITHM,
            "auth_data": self.auth_data(master)?,
            "expected_current_version": expected_current_version,
        }))
    }

    /// `session_data` を復号する。MAC を定数時間で比べてから復号する。
    pub fn decrypt(&self, session_data: &SessionData) -> Result<BackedUpSessionData, CryptoError> {
        let ephemeral = Curve25519PublicKey::from_base64(&session_data.ephemeral)
            .map_err(|_| CryptoError::BackupDecryption("ephemeral が鍵ではありません".into()))?;
        let ciphertext = base64_decode(&session_data.ciphertext).map_err(|_| {
            CryptoError::BackupDecryption("ciphertext が base64 ではありません".into())
        })?;
        let mac = base64_decode(&session_data.mac)
            .map_err(|_| CryptoError::BackupDecryption("mac が base64 ではありません".into()))?;
        let keys = derive_backup_keys(&self.secret, &ephemeral)?;
        let mut hmac = <HmacSha256 as hmac::KeyInit>::new_from_slice(keys.mac_key.as_slice())
            .expect("HMAC はどの長さの鍵も受け付ける");
        hmac.update(&ciphertext);
        hmac.verify_slice(&mac)
            .map_err(|_| CryptoError::BackupDecryption("MAC が一致しません".into()))?;
        let plaintext = Zeroizing::new(
            Aes256CbcDec::new_from_slices(keys.aes_key.as_slice(), &keys.iv)
                .expect("32 バイトの鍵と 16 バイトの IV")
                .decrypt_padded_vec::<Pkcs7>(&ciphertext)
                .map_err(|_| {
                    CryptoError::BackupDecryption("パディングが正しくありません".into())
                })?,
        );
        serde_json::from_slice(&plaintext)
            .map_err(|e| CryptoError::BackupDecryption(format!("平文の形式が違います: {e}")))
    }

    /// `GET /api/e2ee/key_backup/keys` の行を復号し、取り込める形にする。
    ///
    /// 平文の `algorithm` が Megolm でない、`channel_id`、`sender_key`、`session_id` が行の値と違う、
    /// `channel_id` が UUID でないときは [`CryptoError::BackupDecryption`] を返す（呼び出し側は捨てる）。
    /// `session_key` から導いた `session_id` の照合は
    /// [`InboundSessionStore::import_backup_session`] が行う。
    ///
    /// 行の `is_verified` と平文の `sender_*` は、送信者が誰かの判定に使わない（取り込んだセッションは
    /// 必ずバックアップ由来になる）。
    pub fn decrypt_row(&self, row: &BackupRow) -> Result<BackupSession, CryptoError> {
        let data = self.decrypt(&row.session_data)?;
        let mismatch = |field: &str| {
            Err(CryptoError::BackupDecryption(format!(
                "平文の {field} が行の値と違います"
            )))
        };
        if data.algorithm != MEGOLM_ALGORITHM {
            return Err(CryptoError::BackupDecryption(format!(
                "対応していない方式です: {}",
                data.algorithm
            )));
        }
        if data.channel_id != row.channel_id {
            return mismatch("channel_id");
        }
        if data.sender_key != row.sender_key {
            return mismatch("sender_key");
        }
        if data.session_id != row.session_id {
            return mismatch("session_id");
        }
        validate_uuid("channel_id", &data.channel_id, false)
            .map_err(|e| CryptoError::BackupDecryption(e.to_string()))?;
        let bad_key =
            |field: &str| CryptoError::BackupDecryption(format!("{field} が鍵ではありません"));
        Ok(BackupSession {
            sender_key: Curve25519PublicKey::from_base64(&data.sender_key)
                .map_err(|_| bad_key("sender_key"))?,
            sender_signing_key: Ed25519PublicKey::from_base64(&data.sender_claimed_keys.ed25519)
                .map_err(|_| bad_key("sender_claimed_keys.ed25519"))?,
            channel_id: data.channel_id,
            session_id: data.session_id,
            sender_user_id: data.sender_user_id,
            sender_device_id: data.sender_device_id,
            epoch: data.epoch,
            session_key: data.session_key,
        })
    }

    /// 行を復号して、`store` にバックアップ由来として取り込む。捨てた行の数を返す。
    ///
    /// 復号できない行、行と平文が合わない行、`session_key` と `session_id` が合わない行は捨てる。
    pub fn import_rows(&self, store: &mut InboundSessionStore, rows: &[BackupRow]) -> usize {
        rows.iter()
            .filter(|row| {
                self.decrypt_row(row)
                    .and_then(|session| store.import_backup_session(session))
                    .is_err()
            })
            .count()
    }
}

/// 版の公開鍵。セッションを暗号化する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupEncryptionKey(Curve25519PublicKey);

impl BackupEncryptionKey {
    /// 公開鍵から作る。
    pub fn new(public_key: Curve25519PublicKey) -> Self {
        Self(public_key)
    }

    /// 公開鍵。
    pub fn public_key(&self) -> Curve25519PublicKey {
        self.0
    }

    /// セッションを暗号化する。呼ぶたびに新しい一時鍵を作る（同じセッションを上げ直すときも）。
    pub fn encrypt(&self, data: &BackedUpSessionData) -> Result<SessionData, CryptoError> {
        let mut ephemeral: [u8; 32] = random_bytes();
        let result = self.encrypt_with_ephemeral(data, &ephemeral);
        ephemeral.zeroize();
        result
    }

    /// 一時鍵を指定して暗号化する（テストベクター用）。一時鍵を使い回してはならない。
    #[doc(hidden)]
    pub fn encrypt_with_ephemeral(
        &self,
        data: &BackedUpSessionData,
        ephemeral: &[u8; 32],
    ) -> Result<SessionData, CryptoError> {
        let ephemeral = Curve25519SecretKey::from_slice(ephemeral);
        let keys = derive_backup_keys(&ephemeral, &self.0)?;
        let plaintext = Zeroizing::new(
            serde_json::to_vec(data).expect("文字列と数値だけの構造体は必ず JSON にできる"),
        );
        let ciphertext = Aes256CbcEnc::new_from_slices(keys.aes_key.as_slice(), &keys.iv)
            .expect("32 バイトの鍵と 16 バイトの IV")
            .encrypt_padded_vec::<Pkcs7>(&plaintext);
        let mut hmac = <HmacSha256 as hmac::KeyInit>::new_from_slice(keys.mac_key.as_slice())
            .expect("HMAC はどの長さの鍵も受け付ける");
        hmac.update(&ciphertext);
        Ok(SessionData {
            ephemeral: Curve25519PublicKey::from(&ephemeral).to_base64(),
            ciphertext: base64_encode(&ciphertext),
            mac: base64_encode(hmac.finalize().into_bytes()),
        })
    }
}

struct BackupKeys {
    aes_key: Zeroizing<[u8; 32]>,
    mac_key: Zeroizing<[u8; 32]>,
    iv: [u8; 16],
}

/// ECDH の共有秘密から `HKDF-SHA256(salt = 32 バイトの 0, info = "zoo e2ee megolm backup v1")` で
/// 80 バイトを作り、AES 鍵、MAC 鍵、IV に分ける。
fn derive_backup_keys(
    secret: &Curve25519SecretKey,
    public: &Curve25519PublicKey,
) -> Result<BackupKeys, CryptoError> {
    let shared = secret
        .diffie_hellman(public)
        .ok_or_else(|| CryptoError::BackupDecryption("鍵交換の結果が 0 になる公開鍵です".into()))?;
    let mut okm = Zeroizing::new([0u8; 80]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), shared.as_bytes())
        .expand(BACKUP_INFO, okm.as_mut_slice())
        .expect("80 バイトは HKDF-SHA256 の上限より短い");
    let mut keys = BackupKeys {
        aes_key: Zeroizing::new([0u8; 32]),
        mac_key: Zeroizing::new([0u8; 32]),
        iv: [0u8; 16],
    };
    keys.aes_key.copy_from_slice(&okm[..32]);
    keys.mac_key.copy_from_slice(&okm[32..64]);
    keys.iv.copy_from_slice(&okm[64..]);
    Ok(keys)
}

/// 暗号化したセッション（行の `session_data`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionData {
    /// 一時鍵の公開鍵（パディングなしの base64）。
    pub ephemeral: String,
    /// AES-256-CBC の暗号文（パディングなしの base64）。
    pub ciphertext: String,
    /// 暗号文の HMAC-SHA256（32 バイト、パディングなしの base64）。
    pub mac: String,
}

/// 平文の `sender_claimed_keys`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderClaimedKeys {
    /// 送信端末の署名鍵（パディングなしの base64）。
    pub ed25519: String,
}

/// 暗号化する平文（docs/e2ee.md の `BackedUpSessionData` の zoo の形）。
///
/// `Debug` は `session_key` を出さない。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackedUpSessionData {
    /// `m.megolm.v1.aes-sha2`。
    pub algorithm: String,
    /// セッションを使うチャンネル。
    pub channel_id: String,
    /// セッションの epoch。
    pub epoch: u64,
    /// Megolm セッション ID。
    pub session_id: String,
    /// エクスポートした鍵（vodozemac の `ExportedSessionKey` の base64）。
    pub session_key: String,
    /// 送信端末の識別鍵。
    pub sender_key: String,
    /// 送信端末の署名鍵。
    pub sender_claimed_keys: SenderClaimedKeys,
    /// 送信ユーザー。
    pub sender_user_id: String,
    /// 送信端末。
    pub sender_device_id: String,
}

impl std::fmt::Debug for BackedUpSessionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackedUpSessionData")
            .field("algorithm", &self.algorithm)
            .field("channel_id", &self.channel_id)
            .field("epoch", &self.epoch)
            .field("session_id", &self.session_id)
            .field("sender_key", &self.sender_key)
            .field("sender_claimed_keys", &self.sender_claimed_keys)
            .field("sender_user_id", &self.sender_user_id)
            .field("sender_device_id", &self.sender_device_id)
            .finish_non_exhaustive()
    }
}

/// `POST /api/e2ee/key_backup/keys` と `GET /api/e2ee/key_backup/keys` の行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRow {
    /// チャンネル。
    pub channel_id: String,
    /// 送信端末の識別鍵。
    pub sender_key: String,
    /// Megolm セッション ID。
    pub session_id: String,
    /// エクスポートしたインデックス。
    pub first_message_index: u32,
    /// 転送された回数（初期版は常に 0）。
    pub forwarded_count: u32,
    /// 送信端末を検証できたか。サーバーが残す行を決めるためだけに使う。
    pub is_verified: bool,
    /// 暗号化したセッション。
    pub session_data: SessionData,
}

/// サーバーが返した版（`GET /api/e2ee/key_backup/versions/current` などの要素）。検証はしていない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupVersionInfo {
    /// 版の番号（10 進の文字列）。
    pub version: String,
    /// 番号。
    pub number: u64,
    /// `auth_data.public_key`。
    pub public_key: Curve25519PublicKey,
    /// 消した版か（`GET /api/e2ee/key_backup/versions` の `deleted`。ほかの応答では false）。
    pub deleted: bool,
    /// 方式名。
    pub algorithm: Option<String>,
    /// `auth_data`。
    pub auth_data: Value,
}

impl BackupVersionInfo {
    /// 版の JSON を読む。番号が 10 進でない、`auth_data.public_key` が鍵でないときは
    /// [`CryptoError::InvalidBackup`] を返す。
    pub fn from_json(value: &Value) -> Result<Self, CryptoError> {
        let invalid = |message: &str| CryptoError::InvalidBackup(message.to_owned());
        let version = value
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("version がありません"))?;
        let number = parse_version_number(version)?;
        let auth_data = value
            .get("auth_data")
            .cloned()
            .ok_or_else(|| invalid("auth_data がありません"))?;
        let public_key = auth_data
            .get("public_key")
            .and_then(Value::as_str)
            .and_then(|key| Curve25519PublicKey::from_base64(key).ok())
            .ok_or_else(|| invalid("auth_data.public_key が鍵ではありません"))?;
        Ok(Self {
            version: version.to_owned(),
            number,
            public_key,
            deleted: value
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            algorithm: value
                .get("algorithm")
                .and_then(Value::as_str)
                .map(str::to_owned),
            auth_data,
        })
    }

    /// `GET /api/e2ee/key_backup/versions` の応答（`{"versions": [...]}` か配列）を読む。
    pub fn list_from_json(value: &Value) -> Result<Vec<Self>, CryptoError> {
        value
            .get("versions")
            .unwrap_or(value)
            .as_array()
            .ok_or_else(|| CryptoError::InvalidBackup("versions が配列ではありません".into()))?
            .iter()
            .map(Self::from_json)
            .collect()
    }
}

/// 検証を通った今の版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBackupVersion {
    /// 版の番号（10 進の文字列）。
    pub version: String,
    /// 番号。
    pub number: u64,
    /// 公開鍵（SSSS のバックアップ鍵と一致したもの）。
    pub public_key: Curve25519PublicKey,
}

/// 今の版を検証する（docs/e2ee.md の「鍵バックアップ」の【決定】）。
///
/// - `algorithm` があれば [`BACKUP_ALGORITHM`] でなければならない。
/// - `auth_data` に、`user_id` のマスター鍵 `master_key` の署名がなければならない。
/// - `auth_data.public_key` が、SSSS の `m.megolm_backup.v1` から導いた公開鍵と一致しなければならない
///   （違えば [`CryptoError::BackupKeyMismatch`]。呼び出し側は使わずに警告を出す）。
///
/// 今の版と、すべてのアップロードにこの検証を当てる。古い版からの取り込みは
/// [`previous_versions_to_migrate`] で扱う。
pub fn verify_backup_version(
    version: &Value,
    user_id: &str,
    master_key: &Ed25519PublicKey,
    backup_key: &BackupDecryptionKey,
) -> Result<VerifiedBackupVersion, CryptoError> {
    let info = BackupVersionInfo::from_json(version)?;
    if info
        .algorithm
        .as_deref()
        .is_some_and(|algorithm| algorithm != BACKUP_ALGORITHM)
    {
        return Err(CryptoError::InvalidBackup(format!(
            "対応していない方式です: {:?}",
            info.algorithm
        )));
    }
    verify_json_signature(
        &info.auth_data,
        user_id,
        &cross_signing_key_id(master_key),
        master_key,
    )?;
    if info.public_key != backup_key.public_key() {
        return Err(CryptoError::BackupKeyMismatch);
    }
    Ok(VerifiedBackupVersion {
        version: info.version,
        number: info.number,
        public_key: info.public_key,
    })
}

/// 版の番号を読む。`0` で始まらない 10 進の数（`0` を除く）でなければならない。
fn parse_version_number(version: &str) -> Result<u64, CryptoError> {
    let valid = !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_digit())
        && !version.starts_with('0');
    valid
        .then(|| version.parse().ok())
        .flatten()
        .ok_or_else(|| {
            CryptoError::InvalidBackup(format!("version が 10 進の数ではありません: {version:?}"))
        })
}

/// `zoo.megolm_backup.previous` に入れる鍵を選ぶ（docs/e2ee.md の「古い版の秘密鍵の退避」）。
///
/// `candidates` は SSSS の `m.megolm_backup.v1`、すでにある `zoo.megolm_backup.previous` のすべての要素、
/// 失効を行う端末の暗号ストアにあるバックアップ鍵。`versions` は `GET /api/e2ee/key_backup/versions`。
///
/// - 消していない版（今の版を含む）と合う鍵は入れる。
/// - 消した版と合う鍵は除く。
/// - どの版とも合わない鍵は除かずに入れる（別の端末の失効が、その鍵で版を作る途中かもしれない）。
/// - 同じ公開鍵の鍵は 1 つにまとめる。
pub fn select_previous_backup_keys(
    candidates: impl IntoIterator<Item = BackupDecryptionKey>,
    versions: &[BackupVersionInfo],
) -> PreviousBackupSelection {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    for key in candidates {
        let public = key.public_key().to_base64();
        if !seen.insert(public) {
            continue;
        }
        let matching: Vec<_> = versions
            .iter()
            .filter(|v| v.public_key == key.public_key())
            .collect();
        if matching.is_empty() || matching.iter().any(|v| !v.deleted) {
            keys.push(key);
        }
    }
    let versions_without_key = versions
        .iter()
        .filter(|v| !v.deleted && !keys.iter().any(|k| k.public_key() == v.public_key))
        .map(|v| v.version.clone())
        .collect();
    PreviousBackupSelection {
        keys,
        versions_without_key,
    }
}

/// [`select_previous_backup_keys`] の結果。
#[derive(Debug, Clone)]
pub struct PreviousBackupSelection {
    /// `zoo.megolm_backup.previous` に入れる鍵。
    pub keys: Vec<BackupDecryptionKey>,
    /// 消していない版のうち、秘密鍵がどの候補にもない版。失効の画面に「新しい版に移せない
    /// バックアップ」として出す。
    pub versions_without_key: Vec<String>,
}

/// 解錠したあとに移行する古い版と、その鍵を返す（docs/e2ee.md の「古い版の秘密鍵の退避」）。
///
/// `zoo.megolm_backup.previous` の要素のうち、今の版ではない、消していない版と公開鍵が合うもの。
/// これらの版は、署名と `m.megolm_backup.v1` との一致を確かめずに取り込む（取り込んだセッションは
/// バックアップ由来になる）。
pub fn previous_versions_to_migrate<'a>(
    previous_keys: &'a [BackupDecryptionKey],
    versions: &[BackupVersionInfo],
    current_version: Option<&str>,
) -> Vec<(String, &'a BackupDecryptionKey)> {
    versions
        .iter()
        .filter(|v| !v.deleted && Some(v.version.as_str()) != current_version)
        .filter_map(|v| {
            previous_keys
                .iter()
                .find(|k| k.public_key() == v.public_key)
                .map(|k| (v.version.clone(), k))
        })
        .collect()
}

/// 鍵バックアップのアップロードの状態（docs/e2ee.md の「セッションのアップロード」）。
///
/// - アップロードするのは、解錠済みで、今の版を [`verify_backup_version`] で検証できたときだけ
///   （[`accept_current_version`](Self::accept_current_version)）。
/// - 今の版として見た番号の最大を記録し、それより小さい版が今の版として返ったら使わない。
/// - アップロードを待つセッションを記録し、成功するまで残す。
/// - 409 `E2EE_WRONG_BACKUP_VERSION` と 409 `E2EE_SSSS_CONFLICT` を受けたら止める。
///
/// 保存するのは [`to_json`](Self::to_json) の JSON。今の版の検証の結果は保存せず、起動のたびに
/// 検証し直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupUploadState {
    user_id: String,
    max_seen_version: Option<u64>,
    secrets_revision: Option<u64>,
    /// アップロードを待つセッションと、待ちに入れた世代（入れるたびに増やす）。
    pending: BTreeMap<SessionRef, u64>,
    /// 次に使う世代。
    next_generation: u64,
    current: Option<VerifiedBackupVersion>,
}

/// [`BackupUploadState`] の保存形式。
#[derive(Serialize, Deserialize)]
struct BackupUploadStateJson {
    version: u32,
    user_id: String,
    max_seen_version: Option<String>,
    secrets_revision: Option<u64>,
    pending: Vec<SessionRef>,
}

const UPLOAD_STATE_VERSION: u32 = 1;

/// [`BackupUploadState::accept_current_version`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionChange {
    /// 前に受け入れた版と同じ。
    Unchanged,
    /// 新しい版になった（初めての版を含む）。呼び出し側は手元のセッションをすべて
    /// [`enqueue_all`](BackupUploadState::enqueue_all) でアップロード待ちにする。
    Changed,
}

/// 1 回のアップロード（`POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`）。
#[derive(Debug, Clone)]
pub struct BackupUpload {
    /// クエリの `version`。
    pub version: String,
    /// クエリの `secrets_revision`。
    pub secrets_revision: u64,
    /// 要求の本文 `{"sessions": [...]}`。
    pub body: Value,
    /// このアップロードで送るセッション（手元から消えていたものを含む）。
    pub sessions: Vec<SessionRef>,
    /// `sessions` の各要素を待ちに入れた世代。成功したときに、要求を作ったあとで入れ直した
    /// セッションを待ちから外さないために使う。
    generations: Vec<u64>,
}

impl BackupUploadState {
    /// 空の状態を作る。
    pub fn new(user_id: &str) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        Ok(Self {
            user_id: user_id.to_owned(),
            max_seen_version: None,
            secrets_revision: None,
            pending: BTreeMap::new(),
            next_generation: 0,
            current: None,
        })
    }

    /// 保存形式にする。
    pub fn to_json(&self) -> String {
        serde_json::to_string(&BackupUploadStateJson {
            version: UPLOAD_STATE_VERSION,
            user_id: self.user_id.clone(),
            max_seen_version: self.max_seen_version.map(|v| v.to_string()),
            secrets_revision: self.secrets_revision,
            pending: self.pending.keys().cloned().collect(),
        })
        .expect("文字列と数値だけの構造体は必ず JSON にできる")
    }

    /// 保存形式から戻す。`version` が違う、`user_id` が今のユーザーでないときは
    /// [`CryptoError::InvalidBackup`] を返す。
    pub fn from_json(json: &str, user_id: &str) -> Result<Self, CryptoError> {
        let invalid = |message: String| CryptoError::InvalidBackup(message);
        let state: BackupUploadStateJson = serde_json::from_str(json)
            .map_err(|e| invalid(format!("保存した状態の形式が違います: {e}")))?;
        if state.version != UPLOAD_STATE_VERSION {
            return Err(invalid(format!("対応していない版です: {}", state.version)));
        }
        if state.user_id != user_id {
            return Err(invalid("別のユーザーの状態です".into()));
        }
        let mut restored = Self::new(user_id)?;
        restored.max_seen_version = state
            .max_seen_version
            .as_deref()
            .map(parse_version_number)
            .transpose()?;
        restored.secrets_revision = state.secrets_revision;
        for session in state.pending {
            restored.enqueue(session);
        }
        Ok(restored)
    }

    /// 解錠したとき（または自分で SSSS を書いたとき）の `secrets_revision` を記録する。
    pub fn set_secrets_revision(&mut self, secrets_revision: Option<u64>) {
        self.secrets_revision = secrets_revision;
    }

    /// 記録した `secrets_revision`。
    pub fn secrets_revision(&self) -> Option<u64> {
        self.secrets_revision
    }

    /// 今の版として見た番号の最大。
    pub fn max_seen_version(&self) -> Option<u64> {
        self.max_seen_version
    }

    /// 受け入れた今の版。
    pub fn current_version(&self) -> Option<&VerifiedBackupVersion> {
        self.current.as_ref()
    }

    /// サーバーが今の版として返した版を検証し、受け入れる。
    ///
    /// 検証は [`verify_backup_version`] と同じ。さらに、今の版として見た番号の最大より小さければ
    /// [`CryptoError::BackupVersionRollback`] を返す。失敗したときはアップロードを止める
    /// （受け入れていた版も外す）。
    pub fn accept_current_version(
        &mut self,
        version: &Value,
        master_key: &Ed25519PublicKey,
        backup_key: &BackupDecryptionKey,
    ) -> Result<VersionChange, CryptoError> {
        let verified = match verify_backup_version(version, &self.user_id, master_key, backup_key) {
            Ok(verified) => verified,
            Err(e) => {
                self.current = None;
                return Err(e);
            }
        };
        if let Some(max) = self.max_seen_version
            && verified.number < max
        {
            self.current = None;
            return Err(CryptoError::BackupVersionRollback {
                max_seen: max,
                received: verified.number,
            });
        }
        let changed = self.max_seen_version != Some(verified.number);
        self.max_seen_version = Some(verified.number);
        self.current = Some(verified);
        Ok(if changed {
            VersionChange::Changed
        } else {
            VersionChange::Unchanged
        })
    }

    /// アップロードを止める（409 `E2EE_WRONG_BACKUP_VERSION`、409 `E2EE_SSSS_CONFLICT`、今の版が 404 の
    /// とき）。アップロード待ちは残す。再開するには、版を取り直して
    /// [`accept_current_version`](Self::accept_current_version) を通す。
    pub fn suspend(&mut self) {
        self.current = None;
    }

    /// セッションをアップロード待ちにする（送信用セッションを作ったとき、Olm で受け取ったとき、
    /// first known index のより小さいものに置き換わったとき）。バックアップから取り込んだセッションは
    /// 入れない（同じ版には上げ直さない）。
    ///
    /// すでに待っているセッションを入れ直すと、新しい世代になる。送っている途中の要求が成功しても、
    /// 入れ直したセッションは待ちに残り、次のアップロードで今の鍵を送る。
    pub fn enqueue(&mut self, session: SessionRef) {
        self.next_generation += 1;
        self.pending.insert(session, self.next_generation);
    }

    /// 手元のセッションをすべてアップロード待ちにする（今の版が新しい版に変わったとき）。
    pub fn enqueue_all(&mut self, store: &InboundSessionStore) {
        for session in store.session_refs() {
            self.enqueue(session);
        }
    }

    /// アップロードを待つセッションの数。
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 次のアップロードを作る。今の版を受け入れていない、`secrets_revision` を記録していない、
    /// 待つセッションがないときは `Ok(None)`。1 回に送るのは `limit`（最大 1000）件まで。
    pub fn next_upload(
        &self,
        store: &InboundSessionStore,
        limit: usize,
    ) -> Result<Option<BackupUpload>, CryptoError> {
        let (Some(current), Some(secrets_revision)) = (&self.current, self.secrets_revision) else {
            return Ok(None);
        };
        let limit = limit.clamp(1, MAX_UPLOAD_SESSIONS);
        let encryption_key = BackupEncryptionKey::new(current.public_key);
        let (sessions, generations): (Vec<SessionRef>, Vec<u64>) = self
            .pending
            .iter()
            .take(limit)
            .map(|(session, generation)| (session.clone(), *generation))
            .unzip();
        if sessions.is_empty() {
            return Ok(None);
        }
        let mut rows = Vec::with_capacity(sessions.len());
        for session in &sessions {
            let Some(exported) = store.export_for_backup(session) else {
                continue;
            };
            rows.push(BackupRow {
                channel_id: session.channel_id.clone(),
                sender_key: session.sender_key.clone(),
                session_id: session.session_id.clone(),
                first_message_index: exported.first_message_index,
                forwarded_count: 0,
                is_verified: exported.is_verified,
                session_data: encryption_key.encrypt(&exported.data)?,
            });
        }
        Ok(Some(BackupUpload {
            version: current.version.clone(),
            secrets_revision,
            body: json!({ "sessions": rows }),
            sessions,
            generations,
        }))
    }

    /// アップロードが成功したら、送ったセッションを待ちから外す。今の版が変わっていたら何もしない
    /// （新しい版にも上げ直す必要があるため）。要求を作ったあとで [`enqueue`](Self::enqueue) で
    /// 入れ直したセッション（first known index のより小さい鍵に置き換わったものなど）は外さない。
    pub fn upload_succeeded(&mut self, upload: &BackupUpload) {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.version == upload.version)
        {
            for (session, generation) in upload.sessions.iter().zip(&upload.generations) {
                if self.pending.get(session) == Some(generation) {
                    self.pending.remove(session);
                }
            }
        }
    }
}
