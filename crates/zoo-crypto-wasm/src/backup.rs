//! 鍵バックアップ（#351）。

use js_sys::Array;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::convert::{IntoJs, bytes32, ed25519, from_js, set, to_js};
use crate::cross_signing::CrossSigningKeyPair;
use crate::megolm::{BackupSession, InboundSessionStore};

/// バックアップ鍵の秘密鍵（Curve25519）。
#[wasm_bindgen]
#[derive(Clone)]
pub struct BackupDecryptionKey {
    pub(crate) inner: core::BackupDecryptionKey,
}

#[wasm_bindgen]
impl BackupDecryptionKey {
    pub fn generate() -> BackupDecryptionKey {
        Self {
            inner: core::BackupDecryptionKey::generate(),
        }
    }

    /// 32 バイトの秘密鍵から作る。
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<BackupDecryptionKey, JsValue> {
        let bytes = bytes32(bytes, "bytes")?;
        Ok(Self {
            inner: core::BackupDecryptionKey::from_bytes(&bytes),
        })
    }

    /// 秘密鍵の 32 バイト。暗号ストアに保存するときだけ使い、使い終わったら消す。
    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.inner.to_bytes().to_vec()
    }

    /// 公開鍵（パディングなしの base64）。
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> String {
        self.inner.public_key().to_base64()
    }

    /// 版の `auth_data`（マスター鍵の署名付き）。
    #[wasm_bindgen(js_name = authData, unchecked_return_type = "JsonObject")]
    pub fn auth_data(&self, master: &CrossSigningKeyPair) -> Result<JsValue, JsValue> {
        to_js(&self.inner.auth_data(&master.inner).js()?)
    }

    /// `POST /api/e2ee/key_backup/versions` の本文。
    #[wasm_bindgen(js_name = createVersionRequest, unchecked_return_type = "JsonObject")]
    pub fn create_version_request(
        &self,
        master: &CrossSigningKeyPair,
        expected_current_version: Option<String>,
    ) -> Result<JsValue, JsValue> {
        to_js(
            &self
                .inner
                .create_version_request(&master.inner, expected_current_version.as_deref())
                .js()?,
        )
    }

    /// `session_data` を復号する。
    #[wasm_bindgen(unchecked_return_type = "BackedUpSessionData")]
    pub fn decrypt(
        &self,
        #[wasm_bindgen(unchecked_param_type = "SessionData")] session_data: JsValue,
    ) -> Result<JsValue, JsValue> {
        let session_data: core::SessionData = from_js(&session_data, "sessionData")?;
        to_js(&self.inner.decrypt(&session_data).js()?)
    }

    /// バックアップの行 1 つを復号し、行と照合する。
    #[wasm_bindgen(js_name = decryptRow)]
    pub fn decrypt_row(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BackupRow")] row: JsValue,
    ) -> Result<BackupSession, JsValue> {
        let row: core::BackupRow = from_js(&row, "row")?;
        Ok(BackupSession {
            inner: self.inner.decrypt_row(&row).js()?,
        })
    }

    /// バックアップの行をまとめて取り込み、**捨てた行の数**を返す（取り込んだ数ではない）。
    /// 復号できない行は飛ばす。0 なら、すべて取り込めた。
    #[wasm_bindgen(js_name = importRows)]
    pub fn import_rows(
        &self,
        store: &mut InboundSessionStore,
        #[wasm_bindgen(unchecked_param_type = "BackupRow[]")] rows: JsValue,
    ) -> Result<usize, JsValue> {
        let rows: Vec<core::BackupRow> = from_js(&rows, "rows")?;
        Ok(self.inner.import_rows(&mut store.inner, &rows))
    }
}

/// バックアップ鍵の公開鍵。
#[wasm_bindgen]
pub struct BackupEncryptionKey {
    inner: core::BackupEncryptionKey,
}

#[wasm_bindgen]
impl BackupEncryptionKey {
    /// 公開鍵（パディングなしの base64）から作る。
    #[wasm_bindgen(constructor)]
    pub fn new(public_key: &str) -> Result<BackupEncryptionKey, JsValue> {
        let key = core::Curve25519PublicKey::from_base64(public_key)
            .map_err(|e| crate::convert::crypto_error(e.into()))?;
        Ok(Self {
            inner: core::BackupEncryptionKey::new(key),
        })
    }

    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> String {
        self.inner.public_key().to_base64()
    }

    /// 平文を暗号化し、`session_data` を返す。
    #[wasm_bindgen(unchecked_return_type = "SessionData")]
    pub fn encrypt(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BackedUpSessionData")] data: JsValue,
    ) -> Result<JsValue, JsValue> {
        let data: core::BackedUpSessionData = from_js(&data, "data")?;
        to_js(&self.inner.encrypt(&data).js()?)
    }
}

/// バックアップ鍵の一覧（`zoo.megolm_backup.previous` など）。
#[wasm_bindgen]
#[derive(Default)]
pub struct BackupKeyList {
    pub(crate) inner: Vec<core::BackupDecryptionKey>,
}

#[wasm_bindgen]
impl BackupKeyList {
    #[wasm_bindgen(constructor)]
    pub fn new() -> BackupKeyList {
        Self::default()
    }

    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.inner.len()
    }

    /// 鍵を足す（写しを入れるので、渡した `key` はそのまま使える）。
    pub fn push(&mut self, key: &BackupDecryptionKey) {
        self.inner.push(key.inner.clone());
    }

    /// `index` 番目の鍵の写し。
    pub fn get(&self, index: usize) -> Option<BackupDecryptionKey> {
        self.inner.get(index).map(|inner| BackupDecryptionKey {
            inner: inner.clone(),
        })
    }
}

fn versions(response: &JsValue) -> Result<Vec<core::BackupVersionInfo>, JsValue> {
    let response: Value = from_js(response, "versions")?;
    core::BackupVersionInfo::list_from_json(&response).js()
}

fn verified_version(v: &core::VerifiedBackupVersion) -> Value {
    json!({"version": v.version, "number": v.number, "public_key": v.public_key.to_base64()})
}

/// 今の版（`GET /api/e2ee/key_backup/version`）を、マスター鍵の署名と SSSS のバックアップ鍵で検証する。
#[wasm_bindgen(js_name = verifyBackupVersion, unchecked_return_type = "VerifiedBackupVersion")]
pub fn verify_backup_version(
    #[wasm_bindgen(unchecked_param_type = "JsonObject")] version: JsValue,
    user_id: &str,
    master_key: &str,
    backup_key: &BackupDecryptionKey,
) -> Result<JsValue, JsValue> {
    let version: Value = from_js(&version, "version")?;
    let verified =
        core::verify_backup_version(&version, user_id, &ed25519(master_key)?, &backup_key.inner)
            .js()?;
    to_js(&verified_version(&verified))
}

/// 失効のときに `zoo.megolm_backup.previous` に入れる鍵を選ぶ。
/// `versions` は `GET /api/e2ee/key_backup/versions` の応答。
/// `{keys: BackupKeyList, versions_without_key: string[]}` を返す。
#[wasm_bindgen(js_name = selectPreviousBackupKeys, unchecked_return_type = "PreviousBackupSelection")]
pub fn select_previous_backup_keys(
    candidates: &BackupKeyList,
    #[wasm_bindgen(unchecked_param_type = "JsonObject | JsonObject[]")] versions_response: JsValue,
) -> Result<JsValue, JsValue> {
    let selection =
        core::select_previous_backup_keys(candidates.inner.clone(), &versions(&versions_response)?);
    let out = to_js(&json!({"versions_without_key": selection.versions_without_key}))?;
    set(
        &out,
        "keys",
        &BackupKeyList {
            inner: selection.keys,
        }
        .into(),
    )?;
    Ok(out)
}

/// 解錠したあとに移行する古い版と鍵 `{version, key}` の配列を返す。
#[wasm_bindgen(js_name = previousVersionsToMigrate, unchecked_return_type = "{ version: string; key: BackupDecryptionKey }[]")]
pub fn previous_versions_to_migrate(
    previous_keys: &BackupKeyList,
    #[wasm_bindgen(unchecked_param_type = "JsonObject | JsonObject[]")] versions_response: JsValue,
    current_version: Option<String>,
) -> Result<JsValue, JsValue> {
    let versions = versions(&versions_response)?;
    let out = Array::new();
    for (version, key) in core::previous_versions_to_migrate(
        &previous_keys.inner,
        &versions,
        current_version.as_deref(),
    ) {
        let item = to_js(&json!({"version": version}))?;
        set(
            &item,
            "key",
            &BackupDecryptionKey { inner: key.clone() }.into(),
        )?;
        out.push(&item);
    }
    Ok(out.into())
}

/// 1 回のアップロード（`POST /api/e2ee/key_backup/keys?version=<版>&secrets_revision=<数>`）。
#[wasm_bindgen]
pub struct BackupUpload {
    inner: core::BackupUpload,
}

#[wasm_bindgen]
impl BackupUpload {
    #[wasm_bindgen(getter)]
    pub fn version(&self) -> String {
        self.inner.version.clone()
    }

    #[wasm_bindgen(getter, js_name = secretsRevision)]
    pub fn secrets_revision(&self) -> f64 {
        self.inner.secrets_revision as f64
    }

    /// 要求の本文 `{sessions: [...]}`。
    #[wasm_bindgen(getter, unchecked_return_type = "JsonObject")]
    pub fn body(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.body)
    }

    #[wasm_bindgen(getter, unchecked_return_type = "SessionRef[]")]
    pub fn sessions(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.sessions)
    }
}

/// 鍵バックアップのアップロード待ちの状態機械。
#[wasm_bindgen]
pub struct BackupUploadState {
    inner: core::BackupUploadState,
}

#[wasm_bindgen]
impl BackupUploadState {
    #[wasm_bindgen(constructor)]
    pub fn new(user_id: &str) -> Result<BackupUploadState, JsValue> {
        Ok(Self {
            inner: core::BackupUploadState::new(user_id).js()?,
        })
    }

    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(json: &str, user_id: &str) -> Result<BackupUploadState, JsValue> {
        Ok(Self {
            inner: core::BackupUploadState::from_json(json, user_id).js()?,
        })
    }

    /// 保存する文字列（JSON）。
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }

    #[wasm_bindgen(js_name = setSecretsRevision)]
    pub fn set_secrets_revision(&mut self, secrets_revision: Option<f64>) -> Result<(), JsValue> {
        self.inner
            .set_secrets_revision(crate::convert::safe_u64_opt(
                secrets_revision,
                "secretsRevision",
            )?);
        Ok(())
    }

    #[wasm_bindgen(getter, js_name = secretsRevision)]
    pub fn secrets_revision(&self) -> Option<f64> {
        self.inner.secrets_revision().map(|r| r as f64)
    }

    #[wasm_bindgen(getter, js_name = maxSeenVersion)]
    pub fn max_seen_version(&self) -> Option<f64> {
        self.inner.max_seen_version().map(|v| v as f64)
    }

    /// 受け入れた今の版。なければ `undefined`。
    #[wasm_bindgen(js_name = currentVersion, unchecked_return_type = "VerifiedBackupVersion | undefined")]
    pub fn current_version(&self) -> Result<JsValue, JsValue> {
        match self.inner.current_version() {
            Some(v) => to_js(&verified_version(v)),
            None => Ok(JsValue::UNDEFINED),
        }
    }

    /// 今の版を検証して受け入れる。`"unchanged"` か `"changed"` を返す。`"changed"` なら
    /// `enqueueAll` で手元のセッションをすべてアップロード待ちにする。
    #[wasm_bindgen(js_name = acceptCurrentVersion, unchecked_return_type = "\"unchanged\" | \"changed\"")]
    pub fn accept_current_version(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] version: JsValue,
        master_key: &str,
        backup_key: &BackupDecryptionKey,
    ) -> Result<String, JsValue> {
        let version: Value = from_js(&version, "version")?;
        let change = self
            .inner
            .accept_current_version(&version, &ed25519(master_key)?, &backup_key.inner)
            .js()?;
        Ok(match change {
            core::VersionChange::Unchanged => "unchanged",
            core::VersionChange::Changed => "changed",
        }
        .to_owned())
    }

    /// アップロードを止める（版が消えた、検証に失敗したなど）。
    pub fn suspend(&mut self) {
        self.inner.suspend();
    }

    /// セッション `{channel_id, sender_key, session_id}` をアップロード待ちにする。
    pub fn enqueue(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "SessionRef")] session: JsValue,
    ) -> Result<(), JsValue> {
        let session: core::SessionRef = from_js(&session, "session")?;
        self.inner.enqueue(session);
        Ok(())
    }

    /// 手元のセッションをすべてアップロード待ちにする。
    #[wasm_bindgen(js_name = enqueueAll)]
    pub fn enqueue_all(&mut self, store: &InboundSessionStore) {
        self.inner.enqueue_all(&store.inner);
    }

    #[wasm_bindgen(getter, js_name = pendingLen)]
    pub fn pending_len(&self) -> usize {
        self.inner.pending_len()
    }

    /// 次のアップロードを作る。送るものがない、またはアップロードを止めているなら `undefined`。
    #[wasm_bindgen(js_name = nextUpload)]
    pub fn next_upload(
        &self,
        store: &InboundSessionStore,
        limit: usize,
    ) -> Result<Option<BackupUpload>, JsValue> {
        Ok(self
            .inner
            .next_upload(&store.inner, limit)
            .js()?
            .map(|inner| BackupUpload { inner }))
    }

    /// アップロードが成功した。
    #[wasm_bindgen(js_name = uploadSucceeded)]
    pub fn upload_succeeded(&mut self, upload: &BackupUpload) {
        self.inner.upload_succeeded(&upload.inner);
    }
}
