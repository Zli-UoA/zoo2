//! 初回設定と解錠のあとに送る要求（#360）。

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::backup::BackupDecryptionKey;
use crate::convert::{IntoJs, bytes32, from_js, to_js};
use crate::cross_signing::{CrossSigningKeyPair, CrossSigningKeys};
use crate::device::DeviceAccount;
use crate::secret_storage::UnlockedSecretStorage;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &'static str = r#"
/** 解錠や初回設定のあとに送る要求。順に送る。 */
export interface ResumePlan {
  /** `POST /api/e2ee/cross_signing/keys` の本文。直前の再認証が要る。 */
  cross_signing: JsonObject | null;
  /** `POST /api/e2ee/cross_signing/signatures` に送る今の端末への署名。 */
  device_signature: SignatureUpload | null;
  /** `POST /api/e2ee/key_backup/versions` の本文（最初の版）。 */
  backup_version: JsonObject | null;
  /** 今の版（すでにあれば）。 */
  current_backup_version: string | null;
}

/** 起動時の確認の結果。 */
export type StartupCheck =
  | { kind: "ready" }
  | { kind: "sign_device"; signature: SignatureUpload }
  | { kind: "resume_setup" }
  | { kind: "unlock" };
"#;

/// SSSS から取り出した（または初回設定で作った）自分の秘密鍵。マスター鍵の秘密鍵を含む。
#[wasm_bindgen]
pub struct OwnSecrets {
    pub(crate) inner: core::OwnSecrets,
}

#[wasm_bindgen]
impl OwnSecrets {
    /// 初回設定で、クロス署名鍵とバックアップ鍵を新しく作る。
    pub fn generate(user_id: &str) -> Result<OwnSecrets, JsValue> {
        Ok(Self {
            inner: core::OwnSecrets {
                cross_signing: core::CrossSigningKeys::generate(user_id).js()?,
                backup_key: core::BackupDecryptionKey::generate(),
            },
        })
    }

    /// 解錠した SSSS から 4 つの秘密を取り出す。
    #[wasm_bindgen(js_name = fromUnlocked)]
    pub fn from_unlocked(unlocked: &UnlockedSecretStorage) -> Result<OwnSecrets, JsValue> {
        Ok(Self {
            inner: core::OwnSecrets::from_unlocked(&unlocked.inner).js()?,
        })
    }

    /// クロス署名鍵（`SsssKey.keysRequest` に渡す）。
    #[wasm_bindgen(getter, js_name = crossSigning)]
    pub fn cross_signing(&self) -> CrossSigningKeys {
        let keys = &self.inner.cross_signing;
        CrossSigningKeys {
            inner: core::CrossSigningKeys {
                master: CrossSigningKeyPair::duplicate(&keys.master).inner,
                self_signing: CrossSigningKeyPair::duplicate(&keys.self_signing).inner,
                user_signing: CrossSigningKeyPair::duplicate(&keys.user_signing).inner,
            },
        }
    }

    /// バックアップ鍵。
    #[wasm_bindgen(getter, js_name = backupKey)]
    pub fn backup_key(&self) -> BackupDecryptionKey {
        BackupDecryptionKey {
            inner: self.inner.backup_key.clone(),
        }
    }

    /// 自分のマスター鍵の公開鍵。
    #[wasm_bindgen(getter, js_name = masterPublicKey)]
    pub fn master_public_key(&self) -> String {
        self.inner.cross_signing.master.public_key().to_base64()
    }

    /// 暗号ストアに置く鍵（マスター鍵の秘密鍵を除く）。
    #[wasm_bindgen(js_name = ownKeys)]
    pub fn own_keys(&self) -> OwnKeys {
        OwnKeys {
            inner: self.inner.own_keys(),
        }
    }

    /// サーバーの状態（`keys/query` の自分の分と、`GET /api/e2ee/key_backup/versions` の応答）と
    /// 比べ、続きに送る要求を決める。鍵が違えば `code` が `ServerKeyMismatch` のエラーを投げる。
    #[wasm_bindgen(js_name = planResume, unchecked_return_type = "ResumePlan")]
    pub fn plan_resume(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] keys_query: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] versions: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let server = server_state(self.user_id(), &keys_query, &versions)?;
        let device_keys: Value = from_js(&device_keys, "deviceKeys")?;
        plan_json(core::plan_resume(&self.inner, &server, &device_keys).js()?)
    }

    /// E2EE のリセットで、サーバーの今の鍵と版を新しく作った鍵に差し替える要求を作る。
    #[wasm_bindgen(js_name = planReset, unchecked_return_type = "ResumePlan")]
    pub fn plan_reset(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] keys_query: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] versions: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let server = server_state(self.user_id(), &keys_query, &versions)?;
        let device_keys: Value = from_js(&device_keys, "deviceKeys")?;
        plan_json(core::plan_reset(&self.inner, &server, &device_keys).js()?)
    }

    fn user_id(&self) -> &str {
        self.inner.cross_signing.master.user_id()
    }
}

fn plan_json(plan: core::ResumePlan) -> Result<JsValue, JsValue> {
    to_js(&json!({
        "cross_signing": plan.cross_signing,
        "device_signature": plan.device_signature,
        "backup_version": plan.backup_version,
        "current_backup_version": plan.current_backup_version,
    }))
}

fn server_state(
    user_id: &str,
    keys_query: &JsValue,
    versions: &JsValue,
) -> Result<core::ServerKeyState, JsValue> {
    let keys_query: Value = from_js(keys_query, "keysQuery")?;
    let versions: Value = from_js(versions, "versions")?;
    core::ServerKeyState::from_json(user_id, &keys_query, &versions).js()
}

/// 暗号ストアに置く自分の鍵（自己署名鍵、ユーザー署名鍵、バックアップ鍵、マスター鍵の公開鍵）。
#[wasm_bindgen]
pub struct OwnKeys {
    inner: core::OwnKeys,
}

#[wasm_bindgen]
impl OwnKeys {
    /// 暗号ストアの封筒に入れる。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8]) -> Result<String, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(self.inner.pickle(&account.inner, &key))
    }

    /// [`OwnKeys::pickle`] の値を戻す。
    #[wasm_bindgen(js_name = fromPickle)]
    pub fn from_pickle(
        sealed: &str,
        account: &DeviceAccount,
        storage_key: &[u8],
    ) -> Result<OwnKeys, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(Self {
            inner: core::OwnKeys::from_pickle(sealed, &account.inner, &key).js()?,
        })
    }

    #[wasm_bindgen(getter, js_name = masterPublicKey)]
    pub fn master_public_key(&self) -> String {
        self.inner.master_public_key.to_base64()
    }

    #[wasm_bindgen(getter, js_name = selfSigning)]
    pub fn self_signing(&self) -> CrossSigningKeyPair {
        CrossSigningKeyPair::duplicate(&self.inner.self_signing)
    }

    #[wasm_bindgen(getter, js_name = userSigning)]
    pub fn user_signing(&self) -> CrossSigningKeyPair {
        CrossSigningKeyPair::duplicate(&self.inner.user_signing)
    }

    #[wasm_bindgen(getter, js_name = backupKey)]
    pub fn backup_key(&self) -> BackupDecryptionKey {
        BackupDecryptionKey {
            inner: self.inner.backup_key.clone(),
        }
    }

    /// 起動時にすること（署名し直す、初回設定を続ける、解錠し直す）を決める。
    #[wasm_bindgen(js_name = startupCheck, unchecked_return_type = "StartupCheck")]
    pub fn startup_check(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] keys_query: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] versions: JsValue,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let server = server_state(self.inner.self_signing.user_id(), &keys_query, &versions)?;
        let device_keys: Value = from_js(&device_keys, "deviceKeys")?;
        let check = core::startup_check(&self.inner, &server, &device_keys).js()?;
        to_js(&match check {
            core::StartupCheck::Ready => json!({ "kind": "ready" }),
            core::StartupCheck::SignDevice(signature) => {
                json!({ "kind": "sign_device", "signature": signature })
            }
            core::StartupCheck::ResumeSetup => json!({ "kind": "resume_setup" }),
            core::StartupCheck::Unlock => json!({ "kind": "unlock" }),
        })
    }
}
