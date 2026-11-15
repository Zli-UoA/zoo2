//! 鍵バックアップ（#351）。

use std::sync::{Arc, Mutex};

use zoo_crypto as core;

use crate::{
    CrossSigningKeyPair, InboundSessionStore, Result, SessionRef, ZooCryptoError, bytes32,
    ed25519_from_base64, lock, parse_json,
};

/// バックアップ鍵の秘密鍵（[`core::BackupDecryptionKey`]）。
#[derive(uniffi::Object)]
pub struct BackupDecryptionKey {
    pub(crate) inner: core::BackupDecryptionKey,
}

impl BackupDecryptionKey {
    pub(crate) fn wrap(inner: core::BackupDecryptionKey) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

#[uniffi::export]
impl BackupDecryptionKey {
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Self::wrap(core::BackupDecryptionKey::generate())
    }

    #[uniffi::constructor]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Arc<Self>> {
        let bytes = zeroize::Zeroizing::new(bytes32("bytes", &bytes)?);
        Ok(Self::wrap(core::BackupDecryptionKey::from_bytes(&bytes)))
    }

    /// 秘密鍵の 32 バイト（秘密）。
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    /// 公開鍵（base64）。
    pub fn public_key(&self) -> String {
        self.inner.public_key().to_base64()
    }

    /// 版 `version` の `auth_data`（JSON）を作る。方式と版の番号を入れ、マスター鍵で署名する。
    pub fn auth_data(&self, master: &CrossSigningKeyPair, version: u64) -> Result<String> {
        Ok(self.inner.auth_data(&master.inner, version)?.to_string())
    }

    /// `POST /api/e2ee/key_backup/versions` の本文（JSON）。`version` は作る版の番号
    /// （[`next_backup_version`]）。
    pub fn create_version_request(
        &self,
        master: &CrossSigningKeyPair,
        expected_current_version: Option<String>,
        version: u64,
    ) -> Result<String> {
        Ok(self
            .inner
            .create_version_request(&master.inner, expected_current_version.as_deref(), version)?
            .to_string())
    }

    /// `GET /api/e2ee/key_backup/keys` の行（`BackupRow` の JSON の一覧）を復号し、`store` に取り込む。
    /// 復号できない行は飛ばす。返すのは取り込んだ数。
    pub fn import_rows(&self, store: &InboundSessionStore, rows: Vec<String>) -> Result<u64> {
        let rows = rows
            .iter()
            .map(|row| {
                serde_json::from_str::<core::BackupRow>(row).map_err(|e| {
                    ZooCryptoError::InvalidPayload {
                        reason: format!("rows を読めません: {e}"),
                    }
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(self.inner.import_rows(&mut lock(&store.inner), &rows) as u64)
    }
}

/// 検証を通った今の版（[`core::VerifiedBackupVersion`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerifiedBackupVersion {
    pub version: String,
    pub number: u64,
    pub public_key: String,
}

impl From<&core::VerifiedBackupVersion> for VerifiedBackupVersion {
    fn from(v: &core::VerifiedBackupVersion) -> Self {
        Self {
            version: v.version.clone(),
            number: v.number,
            public_key: v.public_key.to_base64(),
        }
    }
}

/// 今の版（JSON）を検証する。`master_key` は `user_id` のマスター鍵（base64）。
#[uniffi::export]
pub fn verify_backup_version(
    version: String,
    user_id: String,
    master_key: String,
    backup_key: &BackupDecryptionKey,
) -> Result<VerifiedBackupVersion> {
    let version = parse_json("version", &version)?;
    let master_key = ed25519_from_base64(&master_key)?;
    let verified = core::verify_backup_version(&version, &user_id, &master_key, &backup_key.inner)?;
    Ok((&verified).into())
}

/// 次に作る版の番号（`auth_data` に入れて署名する）。`versions` は `GET /api/e2ee/key_backup/versions`
/// の応答（JSON。消した版を含む）。
#[uniffi::export]
pub fn next_backup_version(versions: String) -> Result<u64> {
    Ok(core::next_backup_version(&parse_versions(&versions)?))
}

fn parse_versions(versions: &str) -> Result<Vec<core::BackupVersionInfo>> {
    Ok(core::BackupVersionInfo::list_from_json(&parse_json(
        "versions", versions,
    )?)?)
}

/// [`select_previous_backup_keys`] の結果。
#[derive(uniffi::Record)]
pub struct PreviousBackupSelection {
    /// `zoo.megolm_backup.previous` に入れる鍵。
    pub keys: Vec<Arc<BackupDecryptionKey>>,
    /// 秘密鍵がどの候補にもない、消していない版。
    pub versions_without_key: Vec<String>,
}

/// `zoo.megolm_backup.previous` に入れる鍵を選ぶ。`versions` は `GET /api/e2ee/key_backup/versions` の応答（JSON）。
#[uniffi::export]
pub fn select_previous_backup_keys(
    candidates: Vec<Arc<BackupDecryptionKey>>,
    versions: String,
) -> Result<PreviousBackupSelection> {
    let versions = parse_versions(&versions)?;
    let selection =
        core::select_previous_backup_keys(candidates.iter().map(|k| k.inner.clone()), &versions);
    Ok(PreviousBackupSelection {
        keys: selection
            .keys
            .into_iter()
            .map(BackupDecryptionKey::wrap)
            .collect(),
        versions_without_key: selection.versions_without_key,
    })
}

/// 解錠したあとに移行する古い版と、その鍵。
#[derive(uniffi::Record)]
pub struct VersionToMigrate {
    pub version: String,
    pub key: Arc<BackupDecryptionKey>,
}

/// 解錠したあとに移行する古い版を返す。`versions` は `GET /api/e2ee/key_backup/versions` の応答（JSON）。
#[uniffi::export]
pub fn previous_versions_to_migrate(
    previous_keys: Vec<Arc<BackupDecryptionKey>>,
    versions: String,
    current_version: Option<String>,
) -> Result<Vec<VersionToMigrate>> {
    let versions = parse_versions(&versions)?;
    let keys: Vec<core::BackupDecryptionKey> =
        previous_keys.iter().map(|k| k.inner.clone()).collect();
    Ok(
        core::previous_versions_to_migrate(&keys, &versions, current_version.as_deref())
            .into_iter()
            .map(|(version, key)| VersionToMigrate {
                version,
                key: BackupDecryptionKey::wrap(key.clone()),
            })
            .collect(),
    )
}

/// [`BackupUploadState::accept_current_version`] の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VersionChange {
    Unchanged,
    /// 新しい版になった。手元のセッションをすべて `enqueue_all` でアップロード待ちにする。
    Changed,
}

/// 1 回のアップロード（[`core::BackupUpload`]）。
#[derive(uniffi::Object)]
pub struct BackupUpload {
    inner: core::BackupUpload,
}

#[uniffi::export]
impl BackupUpload {
    /// クエリの `version`。
    pub fn version(&self) -> String {
        self.inner.version.clone()
    }

    /// クエリの `secrets_revision`。
    pub fn secrets_revision(&self) -> u64 {
        self.inner.secrets_revision
    }

    /// 要求の本文（JSON）。
    pub fn body(&self) -> String {
        self.inner.body.to_string()
    }

    /// 送るセッション。
    pub fn sessions(&self) -> Vec<SessionRef> {
        self.inner
            .sessions
            .iter()
            .cloned()
            .map(Into::into)
            .collect()
    }
}

/// 鍵バックアップのアップロードの状態（[`core::BackupUploadState`]）。
///
/// ロックの順序：`InboundSessionStore` と一緒に使うときは、こちらを先に取る。
#[derive(uniffi::Object)]
pub struct BackupUploadState {
    inner: Mutex<core::BackupUploadState>,
}

#[uniffi::export]
impl BackupUploadState {
    #[uniffi::constructor]
    pub fn new(user_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::BackupUploadState::new(&user_id)?),
        }))
    }

    /// [`to_json`](Self::to_json) で保存したものを戻す。
    #[uniffi::constructor]
    pub fn from_json(json: String, user_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::BackupUploadState::from_json(&json, &user_id)?),
        }))
    }

    /// 保存形式（JSON）。端末アカウントと同じ保存鍵で暗号化して置く。
    pub fn to_json(&self) -> String {
        lock(&self.inner).to_json()
    }

    pub fn set_secrets_revision(&self, secrets_revision: Option<u64>) {
        lock(&self.inner).set_secrets_revision(secrets_revision);
    }

    pub fn secrets_revision(&self) -> Option<u64> {
        lock(&self.inner).secrets_revision()
    }

    pub fn max_seen_version(&self) -> Option<u64> {
        lock(&self.inner).max_seen_version()
    }

    pub fn current_version(&self) -> Option<VerifiedBackupVersion> {
        lock(&self.inner).current_version().map(Into::into)
    }

    /// サーバーが今の版として返した版（JSON）を検証し、受け入れる。
    pub fn accept_current_version(
        &self,
        version: String,
        master_key: String,
        backup_key: &BackupDecryptionKey,
    ) -> Result<VersionChange> {
        let version = parse_json("version", &version)?;
        let master_key = ed25519_from_base64(&master_key)?;
        let change =
            lock(&self.inner).accept_current_version(&version, &master_key, &backup_key.inner)?;
        Ok(match change {
            core::VersionChange::Unchanged => VersionChange::Unchanged,
            core::VersionChange::Changed => VersionChange::Changed,
        })
    }

    /// アップロードを止める。
    pub fn suspend(&self) {
        lock(&self.inner).suspend();
    }

    pub fn enqueue(&self, session: SessionRef) {
        lock(&self.inner).enqueue(session.into());
    }

    pub fn enqueue_all(&self, store: &InboundSessionStore) {
        let mut state = lock(&self.inner);
        state.enqueue_all(&lock(&store.inner));
    }

    pub fn pending_len(&self) -> u64 {
        lock(&self.inner).pending_len() as u64
    }

    /// 次のアップロードを作る。送るものがなければ `null`。`limit` は 1000 まで。
    pub fn next_upload(
        &self,
        store: &InboundSessionStore,
        limit: u64,
    ) -> Result<Option<Arc<BackupUpload>>> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let state = lock(&self.inner);
        let upload = state.next_upload(&lock(&store.inner), limit)?;
        Ok(upload.map(|inner| Arc::new(BackupUpload { inner })))
    }

    pub fn upload_succeeded(&self, upload: &BackupUpload) {
        lock(&self.inner).upload_succeeded(&upload.inner);
    }
}
