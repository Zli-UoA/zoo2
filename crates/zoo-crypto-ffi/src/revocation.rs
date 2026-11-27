//! 失効の手順（#362）。Web の `crates/zoo-crypto-wasm/src/revocation.rs` と同じ API を Android から使う。
//!
//! マスター鍵の秘密鍵と SSSS 鍵は [`Revocation`] の中（Rust のメモリー）にだけ置き、Kotlin には
//! 返さない。暗号ストアには [`Revocation::pickle`]（保存鍵で暗号化した進み具合）と
//! [`Revocation::own_keys_pickle`]（新しい自分の鍵）を置く。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use zoo_crypto as core;

use crate::{
    BackupDecryptionKey, CrossSigningKeyPair, DeviceAccount, OwnKeys, Result, SignatureUpload,
    SsssKey, TrustStore, UnlockedSecretStorage, VersionToMigrate, ZooCryptoError, bytes32, lock,
    parse_json,
};

fn from_json<T: serde::de::DeserializeOwned>(field: &str, json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| ZooCryptoError::InvalidPayload {
        reason: format!("{field} を読めません: {e}"),
    })
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("コアの型は必ず JSON にできる")
}

/// 失効の範囲（[`core::RevocationScope`]）。`DeviceOnly` は端末だけ（1、3、4）、`Full` は 1〜5 のすべて。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RevocationScope {
    DeviceOnly,
    Full,
}

impl From<RevocationScope> for core::RevocationScope {
    fn from(scope: RevocationScope) -> Self {
        match scope {
            RevocationScope::DeviceOnly => Self::DeviceOnly,
            RevocationScope::Full => Self::Full,
        }
    }
}

impl From<core::RevocationScope> for RevocationScope {
    fn from(scope: core::RevocationScope) -> Self {
        match scope {
            core::RevocationScope::DeviceOnly => Self::DeviceOnly,
            core::RevocationScope::Full => Self::Full,
        }
    }
}

/// どこまで保存したか（[`core::RevocationStage`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RevocationStage {
    Prepared,
    SecretStorageSaved,
    CrossSigningSaved,
    SignaturesSaved,
    BackupCreated,
    Migrated,
    Verified,
}

impl From<RevocationStage> for core::RevocationStage {
    fn from(stage: RevocationStage) -> Self {
        match stage {
            RevocationStage::Prepared => Self::Prepared,
            RevocationStage::SecretStorageSaved => Self::SecretStorageSaved,
            RevocationStage::CrossSigningSaved => Self::CrossSigningSaved,
            RevocationStage::SignaturesSaved => Self::SignaturesSaved,
            RevocationStage::BackupCreated => Self::BackupCreated,
            RevocationStage::Migrated => Self::Migrated,
            RevocationStage::Verified => Self::Verified,
        }
    }
}

impl From<core::RevocationStage> for RevocationStage {
    fn from(stage: core::RevocationStage) -> Self {
        match stage {
            core::RevocationStage::Prepared => Self::Prepared,
            core::RevocationStage::SecretStorageSaved => Self::SecretStorageSaved,
            core::RevocationStage::CrossSigningSaved => Self::CrossSigningSaved,
            core::RevocationStage::SignaturesSaved => Self::SignaturesSaved,
            core::RevocationStage::BackupCreated => Self::BackupCreated,
            core::RevocationStage::Migrated => Self::Migrated,
            core::RevocationStage::Verified => Self::Verified,
        }
    }
}

/// 署名し直さなかった（または記録で置き換えた）理由（[`core::ResignNote`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ResignNote {
    NoRecord,
    NotConfirmed,
    PinReplaced,
    SignatureUnverified,
    MasterKeyMismatch,
    StaleConfirmation,
    PinPending,
}

impl From<core::ResignNote> for ResignNote {
    fn from(note: core::ResignNote) -> Self {
        match note {
            core::ResignNote::NoRecord => Self::NoRecord,
            core::ResignNote::NotConfirmed => Self::NotConfirmed,
            core::ResignNote::PinReplaced => Self::PinReplaced,
            core::ResignNote::SignatureUnverified => Self::SignatureUnverified,
            core::ResignNote::MasterKeyMismatch => Self::MasterKeyMismatch,
            core::ResignNote::StaleConfirmation => Self::StaleConfirmation,
            core::ResignNote::PinPending => Self::PinPending,
        }
    }
}

/// [`Revocation::begin`] に渡すサーバーの値（Web の `RevocationInput`）。JSON はどれも文字列。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RevocationInput {
    pub scope: RevocationScope,
    /// 自分だけを問い合わせた `keys/query`。
    pub keys_query: String,
    /// `GET /api/e2ee/key_backup/versions`。
    pub versions: String,
    /// `GET /api/e2ee/tofu_pins` のすべてのピン（1 件ずつの JSON）。
    pub pins: Vec<String>,
    /// ピンか記録がある相手の `keys/query`（`signatures` と `master_keys` を使う）。なければ `null`。
    pub targets_query: Option<String>,
    /// 手順 1 で削除した端末。
    pub deleted_devices: Vec<String>,
}

/// 失効の画面に出す、進み具合と一覧（秘密鍵を含まない。Web の `RevocationSummary`）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RevocationSummary {
    pub scope: RevocationScope,
    pub stage: RevocationStage,
    pub deleted_devices: Vec<String>,
    /// 署名し直すピンの相手。
    pub resigned_users: Vec<String>,
    /// 署名し直すマスター鍵の相手。
    pub resigned_master_keys: Vec<String>,
    /// 署名し直さなかった相手と理由。
    pub notes: HashMap<String, Vec<ResignNote>>,
    /// 新しい版に移せないバックアップ。
    pub versions_without_key: Vec<String>,
    /// 新しい既定の SSSS 鍵（2〜5）。
    pub key_id: String,
    pub new_backup_version: Option<String>,
    /// 新しい版に移せずに捨てた行の数（移した版の合計。再開しても残る）。
    pub dropped_sessions: u64,
    /// この失効を見分ける値（作った自己署名鍵の公開鍵。始めるたびに変わる）。
    pub revocation_id: String,
}

fn summary(state: &core::RevocationState) -> RevocationSummary {
    RevocationSummary {
        scope: state.scope.into(),
        stage: state.stage.into(),
        deleted_devices: state.deleted_devices.clone(),
        resigned_users: state
            .resign
            .pins
            .iter()
            .map(|p| p.target_user_id.clone())
            .collect(),
        resigned_master_keys: state
            .resign
            .master_keys
            .iter()
            .map(|m| m.target_user_id.clone())
            .collect(),
        notes: state
            .resign
            .notes
            .iter()
            .map(|(user, notes)| {
                (
                    user.clone(),
                    notes.iter().copied().map(Into::into).collect(),
                )
            })
            .collect(),
        versions_without_key: state.versions_without_key.clone(),
        key_id: state.key_id.clone(),
        new_backup_version: state.new_backup_version.clone(),
        dropped_sessions: state.dropped_total(),
        revocation_id: state.new_keys.self_signing.clone().unwrap_or_default(),
    }
}

/// [`Revocation::signatures`] の結果（Web の `RevocationSignatures`）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RevocationSignatures {
    /// 今の端末への自己署名。
    pub device: SignatureUpload,
    /// 署名し直したマスター鍵への署名。
    pub master_keys: Vec<SignatureUpload>,
    /// 作り直したピン（`POST /api/e2ee/tofu_pins` に `replace: true` で送る。1 件ずつの JSON）。
    pub pins: Vec<String>,
}

/// [`Revocation::reencrypt_rows`] の結果。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReencryptedRows {
    /// 新しい版に送る行（1 件ずつの JSON）。
    pub rows: Vec<String>,
    /// 捨てた行の数。
    pub dropped: u64,
}

fn open_state(
    sealed: &str,
    account: &DeviceAccount,
    storage_key: &[u8],
) -> Result<core::RevocationState> {
    let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", storage_key)?);
    Ok(core::RevocationState::from_pickle(
        sealed,
        &lock(&account.inner),
        &storage_key,
    )?)
}

/// 暗号ストアに保存した進み具合を、秘密鍵を除いて読む（解錠する前の表示用。Web の `Revocation.peek`）。
#[uniffi::export]
pub fn peek_revocation(
    sealed: String,
    account: &DeviceAccount,
    storage_key: Vec<u8>,
) -> Result<RevocationSummary> {
    Ok(summary(&open_state(&sealed, account, &storage_key)?))
}

/// 進めている失効（[`core::Revocation`]）。マスター鍵の秘密鍵と SSSS 鍵を持つので、終わったら
/// Kotlin で `close()` する。
#[derive(uniffi::Object)]
pub struct Revocation {
    inner: Mutex<core::Revocation>,
}

#[uniffi::export]
impl Revocation {
    /// 失効を始める。署名し直す一覧を集め、新しい鍵を作る。[`pickle`](Self::pickle) で暗号ストアに
    /// 保存してから、サーバーへの保存を始める。`local_backup_key` と `old_user_signing` は暗号ストアの
    /// 自分の鍵（`OwnKeys.backup_key`、`OwnKeys.user_signing`。なければ `null`）。
    #[uniffi::constructor]
    pub fn begin(
        input: RevocationInput,
        unlocked: &UnlockedSecretStorage,
        trust: &TrustStore,
        local_backup_key: Option<Arc<BackupDecryptionKey>>,
        old_user_signing: Option<Arc<CrossSigningKeyPair>>,
    ) -> Result<Arc<Self>> {
        let trust = lock(&trust.inner);
        let keys_query = parse_json("keys_query", &input.keys_query)?;
        let versions = parse_json("versions", &input.versions)?;
        let server = core::ServerKeyState::from_json(trust.user_id(), &keys_query, &versions)?;
        let pins = input
            .pins
            .iter()
            .map(|pin| parse_json("pins", pin))
            .collect::<Result<Vec<_>>>()?;
        let targets = input
            .targets_query
            .as_deref()
            .map(|json| parse_json("targets_query", json))
            .transpose()?
            .unwrap_or(Value::Null);
        let signatures: Vec<core::KeySignature> = match targets.get("signatures") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
                ZooCryptoError::InvalidPayload {
                    reason: format!("signatures を読めません: {e}"),
                }
            })?,
        };
        let master_keys: BTreeMap<String, Value> = match targets.get("master_keys") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
                ZooCryptoError::InvalidPayload {
                    reason: format!("master_keys を読めません: {e}"),
                }
            })?,
        };
        let inner = core::Revocation::begin(core::RevocationInput {
            scope: input.scope.into(),
            unlocked: &unlocked.inner,
            trust: &trust,
            server: &server,
            pins: &pins,
            signatures: &signatures,
            master_keys: &master_keys,
            local_backup_key: local_backup_key.as_ref().map(|key| &key.inner),
            old_user_signing: old_user_signing.as_ref().map(|key| &key.inner),
            deleted_devices: input.deleted_devices,
        })?;
        Ok(Arc::new(Self {
            inner: Mutex::new(inner),
        }))
    }

    /// 暗号ストアに保存した進み具合から、開け直した SSSS で続ける。2〜5 で SSSS を保存する前に
    /// 止まっていたら `RevocationRestart`。
    #[uniffi::constructor]
    pub fn resume(
        sealed: String,
        account: &DeviceAccount,
        storage_key: Vec<u8>,
        unlocked: &UnlockedSecretStorage,
    ) -> Result<Arc<Self>> {
        let state = open_state(&sealed, account, &storage_key)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(core::Revocation::resume(state, &unlocked.inner)?),
        }))
    }

    /// 暗号ストアに置く文字列。保存が 1 つ済むたびに保存し直す。
    ///
    /// ロックは端末アカウント、このオブジェクトの順に取る。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        Ok(lock(&self.inner).state().pickle(&account, &storage_key))
    }

    /// 進み具合と、失効の画面に出す一覧。
    pub fn summary(&self) -> RevocationSummary {
        summary(lock(&self.inner).state())
    }

    /// 保存が 1 つ済んだ。
    pub fn advance(&self, stage: RevocationStage) {
        lock(&self.inner).advance(stage.into());
    }

    /// SSSS 鍵（2〜5 では新しい鍵）。2〜5 の鍵スロット（`new_recovery_key_slot`、
    /// `new_passkey_key_slot`）は、この鍵で作る。
    pub fn ssss_key(&self) -> Arc<SsssKey> {
        Arc::new(SsssKey {
            inner: lock(&self.inner).ssss_key().clone(),
        })
    }

    /// 作った鍵（新しい自己署名鍵、ユーザー署名鍵、バックアップ鍵。マスター鍵の秘密鍵は除く）。
    /// クロス署名鍵を保存したあとに、暗号ストアの自分の鍵にする。
    pub fn own_keys(&self) -> Arc<OwnKeys> {
        Arc::new(OwnKeys {
            inner: lock(&self.inner).secrets().own_keys(),
        })
    }

    /// [`own_keys`](Self::own_keys) を暗号ストアに置く形（`OwnKeys.from_pickle` で戻す）。
    ///
    /// ロックは端末アカウント、このオブジェクトの順に取る。
    pub fn own_keys_pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        let own = lock(&self.inner).secrets().own_keys();
        Ok(own.pickle(&account, &storage_key))
    }

    /// SSSS の保存の要求（JSON。2〜5 は `secret_storage/keys`、端末だけは `secret_storage/secrets`）。
    /// `slots` は新しい SSSS 鍵の鍵スロットの JSON の一覧（端末だけでは空）。
    pub fn secret_storage_request(&self, slots: Vec<String>) -> Result<String> {
        let slots = slots
            .iter()
            .map(|slot| from_json::<core::KeySlot>("slots", slot))
            .collect::<Result<Vec<_>>>()?;
        Ok(lock(&self.inner)
            .secret_storage_request(&slots)?
            .to_string())
    }

    /// SSSS を保存した（応答の、または読み直した `secrets_revision`）。
    pub fn secret_storage_saved(&self, secrets_revision: Option<u64>) {
        lock(&self.inner).secret_storage_saved(secrets_revision);
    }

    /// `POST /api/e2ee/cross_signing/keys` の本文（JSON）。
    pub fn cross_signing_request(&self) -> String {
        lock(&self.inner).cross_signing_request().to_string()
    }

    /// 今の端末への署名、署名し直すマスター鍵への署名、作り直すピン。`device_keys` は今の端末の
    /// 端末鍵（`DeviceAccount.device_keys`）。
    pub fn signatures(&self, device_keys: String) -> Result<RevocationSignatures> {
        let device_keys = parse_json("device_keys", &device_keys)?;
        let signatures = lock(&self.inner).signatures(&device_keys)?;
        Ok(RevocationSignatures {
            device: signatures.device.into(),
            master_keys: signatures.master_keys.into_iter().map(Into::into).collect(),
            pins: signatures.pins.iter().map(Value::to_string).collect(),
        })
    }

    /// `POST /api/e2ee/key_backup/versions` の本文（JSON）。
    pub fn backup_version_request(&self) -> Result<String> {
        Ok(lock(&self.inner).backup_version_request()?.to_string())
    }

    /// 新しい版を作った。
    pub fn backup_created(&self, version: String) {
        lock(&self.inner).backup_created(version);
    }

    /// 新しい版に移す古い版と、その鍵（`versions` は読み直した `GET /api/e2ee/key_backup/versions`）。
    pub fn versions_to_migrate(&self, versions: String) -> Result<Vec<VersionToMigrate>> {
        let versions =
            core::BackupVersionInfo::list_from_json(&parse_json("versions", &versions)?)?;
        Ok(lock(&self.inner)
            .versions_to_migrate(&versions)
            .into_iter()
            .map(|(version, key)| VersionToMigrate {
                version,
                key: BackupDecryptionKey::wrap(key),
            })
            .collect())
    }

    /// 新しい版 `version` へのアップロードの証明（`X-Zoo-Backup-Proof` の値）。新しいバックアップ鍵で
    /// 作る（移した行を送るときに使う）。`upload_key` は新しい版の `upload_key`（base64）。
    pub fn upload_proof(
        &self,
        upload_key: String,
        user_id: String,
        version: String,
    ) -> Result<String> {
        let upload_key =
            core::Curve25519PublicKey::from_base64(&upload_key).map_err(core::CryptoError::from)?;
        Ok(lock(&self.inner)
            .secrets()
            .backup_key
            .upload_proof(&upload_key, &user_id, &version)?)
    }

    /// 古い版の行（1 件ずつの JSON）を、新しい版の公開鍵で暗号化し直す。
    pub fn reencrypt_rows(
        &self,
        old_key: &BackupDecryptionKey,
        rows: Vec<String>,
    ) -> Result<ReencryptedRows> {
        let rows = rows
            .iter()
            .map(|row| from_json::<core::BackupRow>("rows", row))
            .collect::<Result<Vec<_>>>()?;
        let (rows, dropped) = lock(&self.inner).reencrypt_rows(&old_key.inner, &rows);
        Ok(ReencryptedRows {
            rows: rows.iter().map(to_json).collect(),
            dropped: dropped as u64,
        })
    }

    /// 署名の保存の前に、相手の今のマスター鍵（`keys/query` の `master_keys` の JSON オブジェクト）と
    /// 照合し、作り直された相手のマスター鍵への署名を外す。外した相手を返す（外したら暗号ストアに
    /// 保存する）。
    pub fn drop_stale_master_keys(&self, master_keys: String) -> Result<Vec<String>> {
        let master_keys: BTreeMap<String, Value> = from_json("master_keys", &master_keys)?;
        Ok(lock(&self.inner).drop_stale_master_keys(&master_keys))
    }

    /// 古い版 `version` で捨てた行の数を記録する（同じ版は置き換える）。古い版を消す前に保存する。
    pub fn record_dropped(&self, version: String, dropped: u64) {
        let dropped = usize::try_from(dropped).unwrap_or(usize::MAX);
        lock(&self.inner).record_dropped(&version, dropped);
    }

    /// 読み直した SSSS（`GET /api/e2ee/secret_storage`）に、この失効の秘密が保存されているか（SSSS の
    /// 保存で 409 を受けたあと、応答をなくした自分の書き込みが済んでいたかを見る）。
    pub fn secret_storage_matches(&self, secret_storage: String) -> Result<bool> {
        let secret_storage = parse_json("secret_storage", &secret_storage)?;
        Ok(lock(&self.inner).secret_storage_matches(&secret_storage)?)
    }

    /// 保存のあとに読み直した値（`GET /api/e2ee/secret_storage`、自分だけの `keys/query`、
    /// `GET /api/e2ee/key_backup/versions/current`）を、作った鍵と比べる。違えば `ServerKeyMismatch`。
    pub fn verify(
        &self,
        secret_storage: String,
        keys_query: String,
        current_version: String,
    ) -> Result<()> {
        let secret_storage = parse_json("secret_storage", &secret_storage)?;
        let keys_query = parse_json("keys_query", &keys_query)?;
        let current_version = parse_json("current_version", &current_version)?;
        Ok(lock(&self.inner).verify(&secret_storage, &keys_query, &current_version)?)
    }

    /// クロス署名鍵を保存したあとに、失効を行う端末の記録を新しい鍵にそろえる。
    ///
    /// ロックはこのオブジェクト、`trust` の順に取る。
    pub fn apply_to_trust(&self, trust: &TrustStore) -> Result<()> {
        let revocation = lock(&self.inner);
        let keys = &revocation.secrets().cross_signing;
        Ok(lock(&trust.inner).apply_revocation(
            &keys.master,
            &keys.user_signing,
            &revocation.state().resign,
        )?)
    }
}
