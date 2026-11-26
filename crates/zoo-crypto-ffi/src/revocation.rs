//! 失効の手順（#362、iOS は #685）。
//!
//! Web の `crates/zoo-crypto-wasm/src/revocation.rs` と同じ形。I/O は持たず、サーバーに送る要求の本文（JSON）と、
//! 暗号ストアに置く進み具合（`pickle`）を作る。マスター鍵の秘密鍵と SSSS 鍵を持つので、終わったら
//! `close()` する。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{Value, json};
use zoo_crypto as core;

use crate::setup::server_state;
use crate::{
    BackupDecryptionKey, CrossSigningKeyPair, DeviceAccount, Result, SsssKey, TrustStore,
    UnlockedSecretStorage, ZooCryptoError, bytes32, lock, parse_json,
};

#[derive(Deserialize)]
struct InputJson {
    scope: core::RevocationScope,
    keys_query: Value,
    versions: Value,
    #[serde(default)]
    pins: Vec<Value>,
    #[serde(default)]
    targets_query: Value,
    #[serde(default)]
    deleted_devices: Vec<String>,
}

fn invalid(reason: String) -> ZooCryptoError {
    ZooCryptoError::InvalidPayload { reason }
}

/// 進めている失効。
#[derive(uniffi::Object)]
pub struct Revocation {
    inner: Mutex<core::Revocation>,
}

impl Revocation {
    fn wrap(inner: core::Revocation) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(inner),
        })
    }
}

fn summary(state: &core::RevocationState) -> String {
    let notes: serde_json::Map<String, Value> = state
        .resign
        .notes
        .iter()
        .map(|(user, notes)| (user.clone(), json!(notes)))
        .collect();
    json!({
        "scope": state.scope,
        "stage": state.stage,
        "deleted_devices": state.deleted_devices,
        "resigned_users": state.resign.pins.iter().map(|p| &p.target_user_id).collect::<Vec<_>>(),
        "resigned_master_keys": state
            .resign
            .master_keys
            .iter()
            .map(|m| &m.target_user_id)
            .collect::<Vec<_>>(),
        "notes": notes,
        "versions_without_key": state.versions_without_key,
        "key_id": state.key_id,
        "new_backup_version": state.new_backup_version,
        "dropped_sessions": state.dropped_total(),
        "revocation_id": state.new_keys.self_signing,
    })
    .to_string()
}

/// 暗号ストアに保存した進み具合を、秘密鍵を除いて読む（解錠する前の表示用。`RevocationSummary` の JSON）。
#[uniffi::export]
pub fn peek_revocation(
    sealed: String,
    account: &DeviceAccount,
    storage_key: Vec<u8>,
) -> Result<String> {
    let key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
    let state = core::RevocationState::from_pickle(&sealed, &lock(&account.inner), &key)?;
    Ok(summary(&state))
}

/// 暗号ストアに保存した進み具合から、開け直した SSSS で続ける。2〜5 で SSSS を保存する前に止まっていたら
/// `RevocationRestart`。
#[uniffi::export]
pub fn resume_revocation(
    sealed: String,
    account: &DeviceAccount,
    storage_key: Vec<u8>,
    unlocked: &UnlockedSecretStorage,
) -> Result<Arc<Revocation>> {
    let key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
    let state = core::RevocationState::from_pickle(&sealed, &lock(&account.inner), &key)?;
    Ok(Revocation::wrap(core::Revocation::resume(
        state,
        &unlocked.inner,
    )?))
}

/// 失効を始める。署名し直す一覧を集め、新しい鍵を作る。`pickle` で暗号ストアに保存してから、サーバーへの保存を
/// 始める。`input_json` は `{scope, keys_query, versions, pins, targets_query, deleted_devices}`。
/// （UniFFI の constructor は自分しか返せないが、これは `Arc<Revocation>` なので、関数にして `begin` とそろえる。）
#[uniffi::export]
pub fn begin_revocation(
    input_json: String,
    unlocked: &UnlockedSecretStorage,
    trust: &TrustStore,
    local_backup_key: Option<Arc<BackupDecryptionKey>>,
    old_user_signing: Option<Arc<CrossSigningKeyPair>>,
) -> Result<Arc<Revocation>> {
    let input: InputJson = serde_json::from_value(parse_json("input", &input_json)?)
        .map_err(|e| invalid(format!("input が正しくありません: {e}")))?;
    let trust = lock(&trust.inner);
    let user_id = trust.user_id().to_owned();
    let server = server_state(
        &user_id,
        &input.keys_query.to_string(),
        &input.versions.to_string(),
    )?;
    let signatures: Vec<core::KeySignature> = match input.targets_query.get("signatures") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| invalid(format!("signatures を読めません: {e}")))?,
    };
    let master_keys: BTreeMap<String, Value> = match input.targets_query.get("master_keys") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| invalid(format!("master_keys を読めません: {e}")))?,
    };
    Ok(Revocation::wrap(core::Revocation::begin(
        core::RevocationInput {
            scope: input.scope,
            unlocked: &unlocked.inner,
            trust: &trust,
            server: &server,
            pins: &input.pins,
            signatures: &signatures,
            master_keys: &master_keys,
            local_backup_key: local_backup_key.as_ref().map(|key| &key.inner),
            old_user_signing: old_user_signing.as_ref().map(|key| &key.inner),
            deleted_devices: input.deleted_devices,
        },
    )?))
}

/// 新しい版に移す古い版と、その鍵。
#[derive(uniffi::Record)]
pub struct RevocationMigration {
    pub version: String,
    pub key: Arc<BackupDecryptionKey>,
}

/// 古い版の行を新しい版の公開鍵で暗号化し直した結果。
#[derive(uniffi::Record)]
pub struct ReencryptedRows {
    /// 行（JSON の配列）。
    pub rows: String,
    /// 移せずに捨てた行の数。
    pub dropped: u64,
}

#[uniffi::export]
impl Revocation {
    /// 暗号ストアに置く文字列。保存が 1 つ済むたびに保存し直す。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        Ok(lock(&self.inner)
            .state()
            .pickle(&lock(&account.inner), &key))
    }

    /// 進み具合と、失効の画面に出す一覧（`RevocationSummary` の JSON）。
    pub fn summary(&self) -> String {
        summary(lock(&self.inner).state())
    }

    /// 保存が 1 つ済んだ。`stage` は `prepared`、`secret_storage_saved`、`cross_signing_saved`、
    /// `signatures_saved`、`backup_created`、`migrated`、`verified`。
    pub fn advance(&self, stage: String) -> Result<()> {
        let stage: core::RevocationStage = serde_json::from_value(Value::String(stage))
            .map_err(|e| invalid(format!("stage が正しくありません: {e}")))?;
        lock(&self.inner).advance(stage);
        Ok(())
    }

    /// SSSS 鍵（2〜5 では新しい鍵）。2〜5 の鍵スロットは、この鍵で作る。
    pub fn ssss_key(&self) -> Arc<SsssKey> {
        Arc::new(SsssKey {
            inner: lock(&self.inner).ssss_key().clone(),
        })
    }

    /// 作った鍵を、暗号ストアの自分の鍵として保存する文字列（`SetupSession.own_keys_pickle` と同じ）。
    pub fn own_keys_pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let own = lock(&self.inner).secrets().own_keys();
        Ok(own.pickle(&lock(&account.inner), &key))
    }

    /// SSSS の保存の要求（JSON）。`slots` は新しい SSSS 鍵の鍵スロット（JSON。端末だけの失効では空）。
    pub fn secret_storage_request(&self, slots: Vec<String>) -> Result<String> {
        let slots: Vec<core::KeySlot> = slots
            .iter()
            .map(|slot| {
                serde_json::from_value(parse_json("slots", slot)?)
                    .map_err(|e| invalid(format!("slots が正しくありません: {e}")))
            })
            .collect::<Result<_>>()?;
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

    /// 今の端末への署名、署名し直すマスター鍵への署名、作り直すピン（`{device, master_keys, pins}` の JSON。
    /// 署名は `{target_user_id, target_key_id, signer_key_id, signature}`）。
    pub fn signatures(&self, device_keys: String) -> Result<String> {
        let device_keys = parse_json("device_keys", &device_keys)?;
        let s = lock(&self.inner).signatures(&device_keys)?;
        let upload = |s: &core::SignatureUpload| {
            json!({
                "target_user_id": s.target_user_id,
                "target_key_id": s.target_key_id,
                "signer_key_id": s.signer_key_id,
                "signature": s.signature,
            })
        };
        Ok(json!({
            "device": upload(&s.device),
            "master_keys": s.master_keys.iter().map(upload).collect::<Vec<_>>(),
            "pins": s.pins,
        })
        .to_string())
    }

    /// `POST /api/e2ee/key_backup/versions` の本文（JSON）。
    pub fn backup_version_request(&self) -> Result<String> {
        Ok(lock(&self.inner).backup_version_request()?.to_string())
    }

    /// 新しい版を作った。
    pub fn backup_created(&self, version: String) {
        lock(&self.inner).backup_created(version);
    }

    /// 新しい版に移す古い版と、その鍵（`GET /api/e2ee/key_backup/versions` を読み直したもの）。
    pub fn versions_to_migrate(&self, versions: String) -> Result<Vec<RevocationMigration>> {
        let versions = crate::backup::parse_versions(&versions)?;
        Ok(lock(&self.inner)
            .versions_to_migrate(&versions)
            .into_iter()
            .map(|(version, key)| RevocationMigration {
                version,
                key: BackupDecryptionKey::wrap(key),
            })
            .collect())
    }

    /// 新しい版 `version` へのアップロードの証明（`X-Zoo-Backup-Proof` の値）。新しいバックアップ鍵で作る。
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

    /// 古い版の行（JSON の配列）を、新しい版の公開鍵で暗号化し直す。
    pub fn reencrypt_rows(
        &self,
        old_key: &BackupDecryptionKey,
        rows: String,
    ) -> Result<ReencryptedRows> {
        let rows: Vec<core::BackupRow> = serde_json::from_value(parse_json("rows", &rows)?)
            .map_err(|e| invalid(format!("rows が正しくありません: {e}")))?;
        let (rows, dropped) = lock(&self.inner).reencrypt_rows(&old_key.inner, &rows);
        Ok(ReencryptedRows {
            rows: json!(rows).to_string(),
            dropped: dropped as u64,
        })
    }

    /// 署名の保存の前に、相手の今のマスター鍵（`keys/query` の `master_keys`、JSON）と照合し、作り直された
    /// 相手のマスター鍵への署名を外す。外した相手を返す（外したら暗号ストアに保存する）。
    pub fn drop_stale_master_keys(&self, master_keys: String) -> Result<Vec<String>> {
        let master_keys: BTreeMap<String, Value> =
            serde_json::from_value(parse_json("master_keys", &master_keys)?)
                .map_err(|e| invalid(format!("master_keys が正しくありません: {e}")))?;
        Ok(lock(&self.inner).drop_stale_master_keys(&master_keys))
    }

    /// 古い版 `version` で捨てた行の数を記録する（同じ版は置き換える）。古い版を消す前に保存する。
    pub fn record_dropped(&self, version: String, dropped: u32) {
        lock(&self.inner).record_dropped(&version, dropped as usize);
    }

    /// 保存のあとに読み直した値（JSON）を、作った鍵と比べる。違えば `ServerKeyMismatch`。
    pub fn verify(
        &self,
        secret_storage: String,
        keys_query: String,
        current_version: String,
    ) -> Result<()> {
        Ok(lock(&self.inner).verify(
            &parse_json("secret_storage", &secret_storage)?,
            &parse_json("keys_query", &keys_query)?,
            &parse_json("current_version", &current_version)?,
        )?)
    }

    /// クロス署名鍵を保存したあとに、失効を行う端末の記録を新しい鍵にそろえる。
    pub fn apply_to_trust(&self, trust: &TrustStore) -> Result<()> {
        let revocation = lock(&self.inner);
        let keys = &revocation.secrets().cross_signing;
        lock(&trust.inner).apply_revocation(
            &keys.master,
            &keys.user_signing,
            &revocation.state().resign,
        )?;
        Ok(())
    }
}
