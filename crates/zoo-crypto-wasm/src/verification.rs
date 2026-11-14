//! 他のユーザーの絵文字 SAS 検証（#368）。
//!
//! 検証のメッセージ（[`VerificationMessageJson`]）と送るメッセージ（[`VerificationOutgoingJson`]）は、
//! Worker と画面のあいだを行き来するので、クラスではなく JSON にする。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::convert::{IntoJs, ed25519, from_js, invalid_argument, safe_u64, to_js};
use crate::cross_signing::{CrossSigningKeyPair, KeySnapshot, TrustStore};
use crate::device::{DeviceAccount, DeviceKeys, OlmDecrypted, OlmSession};

/// 端末鍵（[`DeviceKeys::to_json`] の形）。
#[derive(Serialize, Deserialize)]
pub(crate) struct DeviceKeysJson {
    user_id: String,
    device_id: String,
    algorithms: Vec<String>,
    curve25519: String,
    ed25519: String,
}

impl From<&core::DeviceKeys> for DeviceKeysJson {
    fn from(keys: &core::DeviceKeys) -> Self {
        Self {
            user_id: keys.user_id.clone(),
            device_id: keys.device_id.clone(),
            algorithms: keys.algorithms.clone(),
            curve25519: keys.curve25519.to_base64(),
            ed25519: keys.ed25519.to_base64(),
        }
    }
}

impl DeviceKeysJson {
    fn into_core(self) -> Result<core::DeviceKeys, JsValue> {
        Ok(core::DeviceKeys {
            user_id: self.user_id,
            device_id: self.device_id,
            algorithms: self.algorithms,
            curve25519: core::Curve25519PublicKey::from_base64(&self.curve25519)
                .map_err(|e| invalid_argument(format!("curve25519 が正しくありません: {e}")))?,
            ed25519: ed25519(&self.ed25519)?,
        })
    }
}

/// 受信時の検証を通った検証のメッセージ。
#[derive(Serialize, Deserialize)]
struct VerificationMessageJson {
    sender_user_id: String,
    sender_device: DeviceKeysJson,
    #[serde(rename = "type")]
    event_type: String,
    content: Value,
}

impl VerificationMessageJson {
    fn into_core(self) -> Result<core::VerificationMessage, JsValue> {
        Ok(core::VerificationMessage {
            sender_user_id: self.sender_user_id,
            sender_device: self.sender_device.into_core()?,
            event_type: self.event_type,
            content: self.content,
        })
    }
}

/// 送るメッセージ。
#[derive(Serialize)]
struct VerificationOutgoingJson {
    devices: Vec<DeviceKeysJson>,
    #[serde(rename = "type")]
    event_type: &'static str,
    content: Value,
}

fn outgoing_js(out: Vec<core::VerificationOutgoing>) -> Result<JsValue, JsValue> {
    let out: Vec<VerificationOutgoingJson> = out
        .into_iter()
        .map(|o| VerificationOutgoingJson {
            devices: o.devices.iter().map(DeviceKeysJson::from).collect(),
            event_type: o.event_type,
            content: o.content,
        })
        .collect();
    to_js(&out)
}

#[wasm_bindgen]
impl OlmDecrypted {
    /// 絵文字 SAS の検証のメッセージ（`m.key.verification.*`）を、受信時の検証の手順 1〜3 で確かめる。
    ///
    /// `listedDeviceKeys` は、平文の `sender` と `sender_device` で探した `keys/query` の応答の
    /// `device_keys`。なければ（`null`）捨てる（`code` は `Rejected`）。
    #[wasm_bindgen(js_name = verifyVerification, unchecked_return_type = "VerificationMessage")]
    pub fn verify_verification(
        &self,
        account: &DeviceAccount,
        #[wasm_bindgen(unchecked_param_type = "JsonObject | null | undefined")]
        listed_device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let listed: Option<Value> =
            crate::convert::from_js_opt(&listed_device_keys, "listedDeviceKeys")?;
        let listed = match &listed {
            Some(value) => core::ListedDevice::Listed(value),
            None => core::ListedDevice::NotListed,
        };
        let message = self
            .inner
            .verify_verification(&account.inner, listed)
            .js()?;
        to_js(&VerificationMessageJson {
            sender_user_id: message.sender_user_id,
            sender_device: DeviceKeysJson::from(&message.sender_device),
            event_type: message.event_type,
            content: message.content,
        })
    }
}

#[wasm_bindgen]
impl DeviceAccount {
    /// 検証のメッセージ（`type` は `m.key.verification.*`）を `recipient` に送る to-device の
    /// `content` を作る。`session` は `recipient` の識別鍵とのセッション。
    #[wasm_bindgen(js_name = encryptVerification, unchecked_return_type = "JsonObject")]
    pub fn encrypt_verification(
        &self,
        session: &mut OlmSession,
        recipient: &DeviceKeys,
        event_type: &str,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] content: JsValue,
    ) -> Result<JsValue, JsValue> {
        let content: Value = from_js(&content, "content")?;
        to_js(
            &self
                .inner
                .encrypt_verification(&mut session.inner, &recipient.inner, event_type, &content)
                .js()?,
        )
    }
}

#[wasm_bindgen]
impl DeviceKeys {
    /// [`SasVerification`] が返した送り先（`DeviceKeysJson`）から作り直す。Worker の境界を越えて
    /// 戻ってきた送り先を暗号化に使うためだけのもので、署名は検証しない（送り先は、
    /// `SasVerification` がクロス署名をたどって選んだもの）。
    #[wasm_bindgen(js_name = fromVerificationTarget)]
    pub fn from_verification_target(
        #[wasm_bindgen(unchecked_param_type = "DeviceKeysJson")] value: JsValue,
    ) -> Result<DeviceKeys, JsValue> {
        let json: DeviceKeysJson = from_js(&value, "value")?;
        Ok(DeviceKeys {
            inner: json.into_core()?,
        })
    }
}

/// 1 つの SAS の検証（`zoo_crypto::SasVerification`）。保存はできない（メモリーにだけ置く）。
#[wasm_bindgen]
pub struct SasVerification {
    inner: core::SasVerification,
}

fn identity(
    account: &DeviceAccount,
    trust: &TrustStore,
) -> Result<core::VerificationIdentity, JsValue> {
    core::VerificationIdentity::new(&account.inner, &trust.inner).js()
}

#[wasm_bindgen]
impl SasVerification {
    /// `theirUserId` に検証を求める。`snapshot` は相手の `keys/query` の応答。
    /// `{verification, outgoing}` を返す（`outgoing` は相手の端末に送る要求）。
    #[wasm_bindgen(unchecked_return_type = "SasRequest")]
    pub fn request(
        account: &DeviceAccount,
        trust: &TrustStore,
        snapshot: &KeySnapshot,
        their_user_id: &str,
        now_ms: f64,
    ) -> Result<JsValue, JsValue> {
        let (inner, out) = core::SasVerification::request(
            identity(account, trust)?,
            &snapshot.inner,
            their_user_id,
            safe_u64(now_ms, "nowMs")?,
        )
        .js()?;
        let result = js_sys::Object::new();
        crate::convert::set(&result, "verification", &JsValue::from(Self { inner }))?;
        crate::convert::set(&result, "outgoing", &outgoing_js(vec![out])?)?;
        Ok(result.into())
    }

    /// 届いた要求から検証を作る。古い要求、たどれない端末からの要求などは投げる（捨てる）。
    #[wasm_bindgen(js_name = fromRequest)]
    pub fn from_request(
        account: &DeviceAccount,
        trust: &TrustStore,
        #[wasm_bindgen(unchecked_param_type = "VerificationMessage")] message: JsValue,
        snapshot: &KeySnapshot,
        now_ms: f64,
    ) -> Result<SasVerification, JsValue> {
        let message: VerificationMessageJson = from_js(&message, "message")?;
        Ok(Self {
            inner: core::SasVerification::from_request(
                identity(account, trust)?,
                &message.into_core()?,
                &snapshot.inner,
                safe_u64(now_ms, "nowMs")?,
            )
            .js()?,
        })
    }

    /// 今の状態（画面に出すもの）。
    #[wasm_bindgen(unchecked_return_type = "SasView")]
    pub fn view(&self) -> Result<JsValue, JsValue> {
        let v = &self.inner;
        let phase = match v.phase() {
            core::SasPhase::Requested => "requested",
            core::SasPhase::Incoming => "incoming",
            core::SasPhase::Exchanging => "exchanging",
            core::SasPhase::Compare => "compare",
            core::SasPhase::Confirmed => "confirmed",
            core::SasPhase::Done => "done",
            core::SasPhase::Cancelled => "cancelled",
        };
        to_js(&json!({
            "transaction_id": v.transaction_id(),
            "their_user_id": v.their_user_id(),
            "their_device_id": v.their_device_id(),
            "we_requested": v.we_requested(),
            "started_at": v.started_at_ms(),
            "phase": phase,
            "emoji": v.emoji_indices(),
            "cancel": v.cancellation().map(|c| json!({"code": c.code, "by_us": c.by_us})),
            "verified_master_key": v.verified_master_key().map(|k| k.to_base64()),
        }))
    }

    /// 要求を受け入れる。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn accept(&mut self) -> Result<JsValue, JsValue> {
        outgoing_js(self.inner.accept().js()?)
    }

    /// 絵文字が一致すると答えた。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn confirm(&mut self) -> Result<JsValue, JsValue> {
        outgoing_js(self.inner.confirm().js()?)
    }

    /// 絵文字が一致しないと答えた。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn mismatch(&mut self) -> Result<JsValue, JsValue> {
        outgoing_js(self.inner.mismatch())
    }

    /// 取りやめる（要求を断る）。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn cancel(&mut self) -> Result<JsValue, JsValue> {
        outgoing_js(self.inner.cancel())
    }

    /// 10 分たっても終わっていなければ取りやめる。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn expire(&mut self, now_ms: f64) -> Result<JsValue, JsValue> {
        outgoing_js(self.inner.expire(safe_u64(now_ms, "nowMs")?))
    }

    /// 届いたメッセージで進める。`snapshot` は送信ユーザーの `keys/query` の応答。
    #[wasm_bindgen(unchecked_return_type = "VerificationOutgoing[]")]
    pub fn receive(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "VerificationMessage")] message: JsValue,
        snapshot: &KeySnapshot,
    ) -> Result<JsValue, JsValue> {
        let message: VerificationMessageJson = from_js(&message, "message")?;
        outgoing_js(self.inner.receive(&message.into_core()?, &snapshot.inner))
    }

    /// 検証できた相手のマスター鍵に署名する（`{target_user_id, master_key, pin, signature}`）。
    /// `masterKey` は署名する直前に `keys/query` で読んだ相手のマスター鍵。
    #[wasm_bindgen(js_name = signMasterKey, unchecked_return_type = "AcceptedMasterKey")]
    pub fn sign_master_key(
        &self,
        trust: &mut TrustStore,
        user_signing: &CrossSigningKeyPair,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] master_key: JsValue,
    ) -> Result<JsValue, JsValue> {
        let master_key: Value = from_js(&master_key, "masterKey")?;
        let accepted = self
            .inner
            .sign_master_key(&mut trust.inner, &user_signing.inner, &master_key)
            .js()?;
        crate::cross_signing::accepted_js(&accepted)
    }
}
