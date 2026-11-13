//! 失効の手順（#362）。

use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::backup::BackupDecryptionKey;
use crate::convert::{IntoJs, bytes32, from_js, invalid_argument, safe_u64_opt, to_js};
use crate::cross_signing::{CrossSigningKeyPair, TrustStore};
use crate::device::DeviceAccount;
use crate::secret_storage::{SsssKey, UnlockedSecretStorage};
use crate::setup::OwnSecrets;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &'static str = r#"
/** 失効の範囲。`device_only` は端末だけ（1、3、4）、`full` は 1〜5 のすべて。 */
export type RevocationScope = "device_only" | "full";

/** どこまで保存したか。 */
export type RevocationStage =
  | "prepared"
  | "secret_storage_saved"
  | "cross_signing_saved"
  | "signatures_saved"
  | "backup_created"
  | "migrated"
  | "verified";

/** 署名し直さなかった（または記録で置き換えた）理由。 */
export type ResignNote =
  | "no_record"
  | "not_confirmed"
  | "pin_replaced"
  | "signature_unverified"
  | "master_key_mismatch";

/** `Revocation.begin` に渡すサーバーの値。 */
export interface RevocationInput {
  scope: RevocationScope;
  /** 自分だけを問い合わせた `keys/query`。 */
  keys_query: JsonObject;
  /** `GET /api/e2ee/key_backup/versions`。 */
  versions: JsonObject;
  /** `GET /api/e2ee/tofu_pins` のすべてのピン。 */
  pins: JsonObject[];
  /** ピンか記録がある相手の `keys/query`（`signatures` と `master_keys` を使う）。 */
  targets_query: JsonObject;
  /** 手順 1 で削除した端末。 */
  deleted_devices: string[];
}

/** 失効の画面に出す、進み具合と一覧（秘密鍵を含まない）。 */
export interface RevocationSummary {
  scope: RevocationScope;
  stage: RevocationStage;
  deleted_devices: string[];
  /** 署名し直すピンの相手。 */
  resigned_users: string[];
  /** 署名し直すマスター鍵の相手。 */
  resigned_master_keys: string[];
  /** 署名し直さなかった相手と理由。 */
  notes: Record<string, ResignNote[]>;
  /** 新しい版に移せないバックアップ。 */
  versions_without_key: string[];
  /** 新しい既定の SSSS 鍵（2〜5）。 */
  key_id: string;
  new_backup_version: string | null;
}

/** `Revocation.signatures` の結果。 */
export interface RevocationSignatures {
  device: SignatureUpload;
  master_keys: SignatureUpload[];
  pins: JsonObject[];
}
"#;

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

/// 進めている失効。マスター鍵の秘密鍵と SSSS 鍵を持つので、終わったら `free()` する。
#[wasm_bindgen]
pub struct Revocation {
    inner: core::Revocation,
}

#[wasm_bindgen]
impl Revocation {
    /// 失効を始める。署名し直す一覧を集め、新しい鍵を作る。`pickle` で暗号ストアに保存してから、
    /// サーバーへの保存を始める。
    pub fn begin(
        #[wasm_bindgen(unchecked_param_type = "RevocationInput")] input: JsValue,
        unlocked: &UnlockedSecretStorage,
        trust: &TrustStore,
        local_backup_key: Option<BackupDecryptionKey>,
    ) -> Result<Revocation, JsValue> {
        let input: InputJson = from_js(&input, "input")?;
        let user_id = trust.inner.user_id().to_owned();
        let server =
            core::ServerKeyState::from_json(&user_id, &input.keys_query, &input.versions).js()?;
        let signatures: Vec<core::KeySignature> = match input.targets_query.get("signatures") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| invalid_argument(format!("signatures を読めません: {e}")))?,
        };
        let master_keys = match input.targets_query.get("master_keys") {
            None | Some(Value::Null) => Default::default(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| invalid_argument(format!("master_keys を読めません: {e}")))?,
        };
        Ok(Self {
            inner: core::Revocation::begin(core::RevocationInput {
                scope: input.scope,
                unlocked: &unlocked.inner,
                trust: &trust.inner,
                server: &server,
                pins: &input.pins,
                signatures: &signatures,
                master_keys: &master_keys,
                local_backup_key: local_backup_key.as_ref().map(|key| &key.inner),
                deleted_devices: input.deleted_devices,
            })
            .js()?,
        })
    }

    /// 暗号ストアに保存した進み具合から、開け直した SSSS で続ける。2〜5 で SSSS を保存する前に
    /// 止まっていたら `code` が `RevocationRestart` のエラー。
    pub fn resume(
        sealed: &str,
        account: &DeviceAccount,
        storage_key: &[u8],
        unlocked: &UnlockedSecretStorage,
    ) -> Result<Revocation, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        let state = core::RevocationState::from_pickle(sealed, &account.inner, &key).js()?;
        Ok(Self {
            inner: core::Revocation::resume(state, &unlocked.inner).js()?,
        })
    }

    /// 暗号ストアに保存した進み具合を、秘密鍵を除いて読む（解錠する前の表示用）。
    #[wasm_bindgen(js_name = peek, unchecked_return_type = "RevocationSummary")]
    pub fn peek(
        sealed: &str,
        account: &DeviceAccount,
        storage_key: &[u8],
    ) -> Result<JsValue, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        let state = core::RevocationState::from_pickle(sealed, &account.inner, &key).js()?;
        summary(&state)
    }

    /// 暗号ストアに置く文字列。保存が 1 つ済むたびに保存し直す。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8]) -> Result<String, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(self.inner.state().pickle(&account.inner, &key))
    }

    /// 進み具合と、失効の画面に出す一覧。
    #[wasm_bindgen(unchecked_return_type = "RevocationSummary")]
    pub fn summary(&self) -> Result<JsValue, JsValue> {
        summary(self.inner.state())
    }

    /// 保存が 1 つ済んだ。
    pub fn advance(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "RevocationStage")] stage: JsValue,
    ) -> Result<(), JsValue> {
        let stage: core::RevocationStage = from_js(&stage, "stage")?;
        self.inner.advance(stage);
        Ok(())
    }

    /// SSSS 鍵（2〜5 では新しい鍵）。2〜5 の鍵スロットは、この鍵で作る。
    #[wasm_bindgen(js_name = ssssKey)]
    pub fn ssss_key(&self) -> SsssKey {
        SsssKey {
            inner: self.inner.ssss_key().clone(),
        }
    }

    /// 作った鍵（新しいマスター鍵を含む）。保存を終えたら暗号ストアの自分の鍵にする。
    #[wasm_bindgen(js_name = ownSecrets)]
    pub fn own_secrets(&self) -> OwnSecrets {
        let secrets = self.inner.secrets();
        let keys = &secrets.cross_signing;
        OwnSecrets {
            inner: core::OwnSecrets {
                cross_signing: core::CrossSigningKeys {
                    master: CrossSigningKeyPair::duplicate(&keys.master).inner,
                    self_signing: CrossSigningKeyPair::duplicate(&keys.self_signing).inner,
                    user_signing: CrossSigningKeyPair::duplicate(&keys.user_signing).inner,
                },
                backup_key: secrets.backup_key.clone(),
            },
        }
    }

    /// SSSS の保存の要求（2〜5 は `secret_storage/keys`、端末だけは `secret_storage/secrets`）。
    #[wasm_bindgen(js_name = secretStorageRequest, unchecked_return_type = "JsonObject")]
    pub fn secret_storage_request(
        &self,
        #[wasm_bindgen(unchecked_param_type = "KeySlot[]")] slots: JsValue,
    ) -> Result<JsValue, JsValue> {
        let slots: Vec<core::KeySlot> = from_js(&slots, "slots")?;
        to_js(&self.inner.secret_storage_request(&slots).js()?)
    }

    /// SSSS を保存した（応答の、または読み直した `secrets_revision`）。
    #[wasm_bindgen(js_name = secretStorageSaved)]
    pub fn secret_storage_saved(&mut self, secrets_revision: Option<f64>) -> Result<(), JsValue> {
        self.inner
            .secret_storage_saved(safe_u64_opt(secrets_revision, "secretsRevision")?);
        Ok(())
    }

    /// `POST /api/e2ee/cross_signing/keys` の本文。
    #[wasm_bindgen(js_name = crossSigningRequest, unchecked_return_type = "JsonObject")]
    pub fn cross_signing_request(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.cross_signing_request())
    }

    /// 今の端末への署名、署名し直すマスター鍵への署名、作り直すピン。
    #[wasm_bindgen(unchecked_return_type = "RevocationSignatures")]
    pub fn signatures(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let device_keys: Value = from_js(&device_keys, "deviceKeys")?;
        let signatures = self.inner.signatures(&device_keys).js()?;
        to_js(&json!({
            "device": signatures.device,
            "master_keys": signatures.master_keys,
            "pins": signatures.pins,
        }))
    }

    /// `POST /api/e2ee/key_backup/versions` の本文。
    #[wasm_bindgen(js_name = backupVersionRequest, unchecked_return_type = "JsonObject")]
    pub fn backup_version_request(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.backup_version_request().js()?)
    }

    /// 新しい版を作った。
    #[wasm_bindgen(js_name = backupCreated)]
    pub fn backup_created(&mut self, version: String) {
        self.inner.backup_created(version);
    }

    /// 新しい版に移す古い版と、その鍵（`GET /api/e2ee/key_backup/versions` を読み直したもの）。
    #[wasm_bindgen(js_name = versionsToMigrate, unchecked_return_type = "{ version: string; key: BackupDecryptionKey }[]")]
    pub fn versions_to_migrate(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] versions: JsValue,
    ) -> Result<JsValue, JsValue> {
        let versions: Value = from_js(&versions, "versions")?;
        let versions = core::BackupVersionInfo::list_from_json(&versions).js()?;
        let out = js_sys::Array::new();
        for (version, key) in self.inner.versions_to_migrate(&versions) {
            let entry = js_sys::Object::new();
            js_sys::Reflect::set(&entry, &"version".into(), &version.into())?;
            js_sys::Reflect::set(
                &entry,
                &"key".into(),
                &BackupDecryptionKey { inner: key }.into(),
            )?;
            out.push(&entry);
        }
        Ok(out.into())
    }

    /// 古い版の行を、新しい版の公開鍵で暗号化し直す。`{rows, dropped}` を返す。
    #[wasm_bindgen(js_name = reencryptRows, unchecked_return_type = "{ rows: JsonObject[]; dropped: number }")]
    pub fn reencrypt_rows(
        &self,
        old_key: &BackupDecryptionKey,
        #[wasm_bindgen(unchecked_param_type = "JsonObject[]")] rows: JsValue,
    ) -> Result<JsValue, JsValue> {
        let rows: Vec<core::BackupRow> = from_js(&rows, "rows")?;
        let (rows, dropped) = self.inner.reencrypt_rows(&old_key.inner, &rows);
        to_js(&json!({ "rows": rows, "dropped": dropped }))
    }

    /// 保存のあとに読み直した値を、作った鍵と比べる。違えば `code` が `ServerKeyMismatch`。
    pub fn verify(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] secret_storage: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] keys_query: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] current_version: JsValue,
    ) -> Result<(), JsValue> {
        let secret_storage: Value = from_js(&secret_storage, "secretStorage")?;
        let keys_query: Value = from_js(&keys_query, "keysQuery")?;
        let current_version: Value = from_js(&current_version, "currentVersion")?;
        self.inner
            .verify(&secret_storage, &keys_query, &current_version)
            .js()
    }

    /// クロス署名鍵を保存したあとに、失効を行う端末の記録を新しい鍵にそろえる。
    #[wasm_bindgen(js_name = applyToTrust)]
    pub fn apply_to_trust(&self, trust: &mut TrustStore) -> Result<(), JsValue> {
        let keys = &self.inner.secrets().cross_signing;
        trust
            .inner
            .apply_revocation(&keys.master, &keys.user_signing, &self.inner.state().resign)
            .js()
    }
}

fn summary(state: &core::RevocationState) -> Result<JsValue, JsValue> {
    let notes: serde_json::Map<String, Value> = state
        .resign
        .notes
        .iter()
        .map(|(user, notes)| (user.clone(), json!(notes)))
        .collect();
    to_js(&json!({
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
    }))
}
