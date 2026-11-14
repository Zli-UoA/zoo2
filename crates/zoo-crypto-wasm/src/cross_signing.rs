//! クロス署名と、配布先の判定（#350）。

use js_sys::Array;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::convert::{IntoJs, bytes32, ed25519, from_js, invalid_argument, set, to_js};
use crate::device::{DeviceAccount, DeviceKeys};

pub(crate) fn parse_usage(usage: &str) -> Result<core::KeyUsage, JsValue> {
    match usage {
        "master" => Ok(core::KeyUsage::Master),
        "self_signing" => Ok(core::KeyUsage::SelfSigning),
        "user_signing" => Ok(core::KeyUsage::UserSigning),
        _ => Err(invalid_argument(format!(
            "usage は master、self_signing、user_signing のどれかでなければなりません: {usage:?}"
        ))),
    }
}

/// クロス署名鍵の秘密鍵 1 つ。
#[wasm_bindgen]
pub struct CrossSigningKeyPair {
    pub(crate) inner: core::CrossSigningKeyPair,
}

impl CrossSigningKeyPair {
    /// 同じ鍵をもう 1 つ作る（コアの型は `Clone` を持たないので、シードから作り直す）。
    pub(crate) fn duplicate(inner: &core::CrossSigningKeyPair) -> Self {
        let seed = inner.seed();
        Self {
            inner: core::CrossSigningKeyPair::from_seed(inner.user_id(), inner.usage(), &seed)
                .expect("元の鍵と同じ user_id なので作り直せる"),
        }
    }
}

#[wasm_bindgen]
impl CrossSigningKeyPair {
    /// 新しい鍵を作る。`usage` は `master`、`self_signing`、`user_signing`。
    pub fn generate(
        user_id: &str,
        #[wasm_bindgen(unchecked_param_type = "KeyUsage")] usage: &str,
    ) -> Result<CrossSigningKeyPair, JsValue> {
        Ok(Self {
            inner: core::CrossSigningKeyPair::generate(user_id, parse_usage(usage)?).js()?,
        })
    }

    /// 32 バイトのシードから作る。
    #[wasm_bindgen(js_name = fromSeed)]
    pub fn from_seed(
        user_id: &str,
        #[wasm_bindgen(unchecked_param_type = "KeyUsage")] usage: &str,
        seed: &[u8],
    ) -> Result<CrossSigningKeyPair, JsValue> {
        let seed = bytes32(seed, "seed")?;
        Ok(Self {
            inner: core::CrossSigningKeyPair::from_seed(user_id, parse_usage(usage)?, &seed)
                .js()?,
        })
    }

    /// 秘密鍵（32 バイトのシード）。暗号ストアに保存するときだけ使い、使い終わったら消す。
    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> String {
        self.inner.user_id().to_owned()
    }

    #[wasm_bindgen(getter, unchecked_return_type = "KeyUsage")]
    pub fn usage(&self) -> String {
        self.inner.usage().as_str().to_owned()
    }

    /// 公開鍵（パディングなしの base64）。
    #[wasm_bindgen(getter, js_name = publicKey)]
    pub fn public_key(&self) -> String {
        self.inner.public_key().to_base64()
    }

    /// 鍵 ID（`ed25519:<公開鍵>`）。
    #[wasm_bindgen(getter, js_name = keyId)]
    pub fn key_id(&self) -> String {
        self.inner.key_id()
    }

    /// 署名なしの公開鍵の JSON（`{user_id, usage, keys}`）。
    #[wasm_bindgen(js_name = publicKeyJson, unchecked_return_type = "JsonObject")]
    pub fn public_key_json(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.public_key_json())
    }

    /// マスター鍵で、自己署名鍵かユーザー署名鍵に署名した JSON を返す。
    #[wasm_bindgen(js_name = signSubkey, unchecked_return_type = "JsonObject")]
    pub fn sign_subkey(&self, subkey: &CrossSigningKeyPair) -> Result<JsValue, JsValue> {
        to_js(&self.inner.sign_subkey(&subkey.inner).js()?)
    }

    /// 自己署名鍵で、自分の端末鍵に署名する。
    #[wasm_bindgen(js_name = signDevice, unchecked_return_type = "SignatureUpload")]
    pub fn sign_device(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] device_keys: JsValue,
    ) -> Result<JsValue, JsValue> {
        let device_keys: Value = from_js(&device_keys, "deviceKeys")?;
        to_js(&self.inner.sign_device(&device_keys).js()?)
    }

    /// ユーザー署名鍵で、他のユーザーのマスター鍵に署名する。
    #[wasm_bindgen(js_name = signMasterKey, unchecked_return_type = "SignatureUpload")]
    pub fn sign_master_key(
        &self,
        target_user_id: &str,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] master_key: JsValue,
    ) -> Result<JsValue, JsValue> {
        let master_key: Value = from_js(&master_key, "masterKey")?;
        to_js(
            &self
                .inner
                .sign_master_key(target_user_id, &master_key)
                .js()?,
        )
    }

    /// ユーザー署名鍵で、TOFU のピン（`zoo.tofu_pin`）を作る。`masterKey` は相手のマスター鍵の公開鍵。
    #[wasm_bindgen(js_name = signTofuPin, unchecked_return_type = "JsonObject")]
    pub fn sign_tofu_pin(
        &self,
        target_user_id: &str,
        master_key: &str,
    ) -> Result<JsValue, JsValue> {
        to_js(
            &self
                .inner
                .sign_tofu_pin(target_user_id, &ed25519(master_key)?)
                .js()?,
        )
    }
}

/// 自分のクロス署名鍵 3 つ（マスター鍵、自己署名鍵、ユーザー署名鍵）。
#[wasm_bindgen]
pub struct CrossSigningKeys {
    pub(crate) inner: core::CrossSigningKeys,
}

#[wasm_bindgen]
impl CrossSigningKeys {
    /// 3 つの鍵を新しく作る。
    pub fn generate(user_id: &str) -> Result<CrossSigningKeys, JsValue> {
        Ok(Self {
            inner: core::CrossSigningKeys::generate(user_id).js()?,
        })
    }

    /// 3 つの鍵から作る（SSSS から取り出した鍵を組み合わせるとき）。渡した鍵はそのまま使える。
    #[wasm_bindgen(js_name = fromKeys)]
    pub fn from_keys(
        master: &CrossSigningKeyPair,
        self_signing: &CrossSigningKeyPair,
        user_signing: &CrossSigningKeyPair,
    ) -> Result<CrossSigningKeys, JsValue> {
        let pairs = [
            (master, core::KeyUsage::Master),
            (self_signing, core::KeyUsage::SelfSigning),
            (user_signing, core::KeyUsage::UserSigning),
        ];
        for (pair, usage) in &pairs {
            if pair.inner.usage() != *usage {
                return Err(invalid_argument(format!(
                    "{} の鍵ではありません",
                    usage.as_str()
                )));
            }
            if pair.inner.user_id() != master.inner.user_id() {
                return Err(invalid_argument("鍵の持ち主が違います"));
            }
        }
        Ok(Self {
            inner: core::CrossSigningKeys {
                master: CrossSigningKeyPair::duplicate(&master.inner).inner,
                self_signing: CrossSigningKeyPair::duplicate(&self_signing.inner).inner,
                user_signing: CrossSigningKeyPair::duplicate(&user_signing.inner).inner,
            },
        })
    }

    #[wasm_bindgen(getter)]
    pub fn master(&self) -> CrossSigningKeyPair {
        CrossSigningKeyPair::duplicate(&self.inner.master)
    }

    #[wasm_bindgen(getter, js_name = selfSigning)]
    pub fn self_signing(&self) -> CrossSigningKeyPair {
        CrossSigningKeyPair::duplicate(&self.inner.self_signing)
    }

    #[wasm_bindgen(getter, js_name = userSigning)]
    pub fn user_signing(&self) -> CrossSigningKeyPair {
        CrossSigningKeyPair::duplicate(&self.inner.user_signing)
    }

    /// `POST /api/e2ee/cross_signing/keys` の本文。
    #[wasm_bindgen(js_name = uploadJson, unchecked_return_type = "JsonObject")]
    pub fn upload_json(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.upload_json())
    }
}

/// サーバーから受け取ったクロス署名鍵を検証し、公開鍵（base64）を返す。
#[wasm_bindgen(js_name = verifyCrossSigningKey)]
pub fn verify_cross_signing_key(
    #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
    user_id: &str,
    #[wasm_bindgen(unchecked_param_type = "KeyUsage")] usage: &str,
) -> Result<String, JsValue> {
    let value: Value = from_js(&value, "value")?;
    Ok(
        core::verify_cross_signing_key(&value, user_id, parse_usage(usage)?)
            .js()?
            .to_base64(),
    )
}

/// 公開鍵（base64）から、クロス署名鍵の鍵 ID（`ed25519:<公開鍵>`）を作る。
#[wasm_bindgen(js_name = crossSigningKeyId)]
pub fn cross_signing_key_id(public_key: &str) -> Result<String, JsValue> {
    Ok(core::cross_signing_key_id(&ed25519(public_key)?))
}

/// `keys/query` と `channels/{id}/recipients` の応答（の積み重ね）。
#[wasm_bindgen]
#[derive(Default)]
pub struct KeySnapshot {
    pub(crate) inner: core::KeySnapshot,
}

#[wasm_bindgen]
impl KeySnapshot {
    /// 空のスナップショット。
    #[wasm_bindgen(constructor)]
    pub fn new() -> KeySnapshot {
        Self::default()
    }

    /// 応答 1 ページを読む。
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
    ) -> Result<KeySnapshot, JsValue> {
        let value: Value = from_js(&value, "value")?;
        Ok(Self {
            inner: core::KeySnapshot::from_json(&value).js()?,
        })
    }

    /// 次のページを足す。渡した `page` は JS からは使えなくなる。
    pub fn extend(&mut self, page: KeySnapshot) {
        self.inner.extend(page.inner);
    }

    /// 含むユーザー ID。
    #[wasm_bindgen(getter, js_name = userIds)]
    pub fn user_ids(&self) -> Vec<String> {
        self.inner.user_ids.clone()
    }
}

#[derive(Deserialize)]
struct NewTofuPinJson {
    target_user_id: String,
    master_key: String,
    pin: Value,
}

impl From<NewTofuPinJson> for core::NewTofuPin {
    fn from(p: NewTofuPinJson) -> Self {
        Self {
            target_user_id: p.target_user_id,
            master_key: p.master_key,
            pin: p.pin,
        }
    }
}

fn user_state(state: core::UserState) -> Value {
    use core::{ReverificationReason as R, TrustBasis as B, UserState as S};
    match state {
        S::Trusted(basis) => json!({
            "state": "trusted",
            "basis": match basis {
                B::Own => "own",
                B::UserSigningSignature => "user_signing_signature",
                B::SharedPin => "shared_pin",
                B::Record => "record",
            },
        }),
        S::NoCrossSigningKeys => json!({"state": "no_cross_signing_keys"}),
        S::InvalidCrossSigningKeys => json!({"state": "invalid_cross_signing_keys"}),
        S::NeedsReverification(reason) => json!({
            "state": "needs_reverification",
            "reason": match reason {
                R::MasterKeyChanged => "master_key_changed",
                R::NotConfirmed => "not_confirmed",
                R::UserSigningKeyChanged => "user_signing_key_changed",
            },
        }),
        S::NeedsUnlock => json!({"state": "needs_unlock"}),
        S::PinPending => json!({"state": "pin_pending"}),
        S::Blocked => json!({"state": "blocked"}),
    }
}

fn device_exclusion(exclusion: core::DeviceExclusion) -> &'static str {
    use core::DeviceExclusion as E;
    match exclusion {
        E::CurrentDevice => "current_device",
        E::InvalidDeviceKeys => "invalid_device_keys",
        E::InvalidSelfSigningKey => "invalid_self_signing_key",
        E::Unsigned => "unsigned",
        E::BadSignature => "bad_signature",
    }
}

fn trust_warning(warning: core::TrustWarning) -> &'static str {
    use core::TrustWarning as W;
    match warning {
        W::OwnIdentityUnknown => "own_identity_unknown",
        W::OwnMasterKeyChanged => "own_master_key_changed",
        W::UntrustedUserSigningKey => "untrusted_user_signing_key",
        W::UserSigningKeyRollback => "user_signing_key_rollback",
        W::UserSigningKeyChanged => "user_signing_key_changed",
    }
}

/// 配布先の判定に使う、暗号ストアの記録（TOFU、`confirmed_usk`、ユーザー署名鍵の履歴）。
#[wasm_bindgen]
pub struct TrustStore {
    pub(crate) inner: core::TrustStore,
}

#[wasm_bindgen]
impl TrustStore {
    #[wasm_bindgen(constructor)]
    pub fn new(user_id: &str) -> Result<TrustStore, JsValue> {
        Ok(Self {
            inner: core::TrustStore::new(user_id).js()?,
        })
    }

    /// [`toJson`](Self::to_json) で保存した記録を読む。
    #[wasm_bindgen(js_name = fromJson)]
    pub fn from_json(json: &str, user_id: &str) -> Result<TrustStore, JsValue> {
        Ok(Self {
            inner: core::TrustStore::from_json(json, user_id).js()?,
        })
    }

    /// 暗号ストアに置く文字列（`toJson` の JSON を、`account` の端末に結び付けて保存鍵で
    /// 暗号化したもの。#359）。記録が変わる操作のあとで保存し直す。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8]) -> Result<String, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(self.inner.pickle(&account.inner, &key))
    }

    /// [`pickle`](Self::pickle) の文字列から戻す。
    #[wasm_bindgen(js_name = fromPickle)]
    pub fn from_pickle(
        pickle: &str,
        account: &DeviceAccount,
        storage_key: &[u8],
    ) -> Result<TrustStore, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(Self {
            inner: core::TrustStore::from_pickle(pickle, &account.inner, &key).js()?,
        })
    }

    /// 保存する文字列（JSON）。記録が変わる操作のあとで保存し直す。
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }

    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> String {
        self.inner.user_id().to_owned()
    }

    /// 信頼した自分のマスター鍵（base64）。
    #[wasm_bindgen(getter, js_name = ownMasterKey)]
    pub fn own_master_key(&self) -> Option<String> {
        self.inner.own_master_key().map(str::to_owned)
    }

    /// 自分のユーザー署名鍵の履歴（base64、古い順）。
    #[wasm_bindgen(getter, js_name = userSigningKeyHistory)]
    pub fn user_signing_key_history(&self) -> Vec<String> {
        self.inner.user_signing_key_history().to_vec()
    }

    /// 相手の記録 `{master_key, confirmed_usk, pin_pending}`。なければ `undefined`。
    #[wasm_bindgen(unchecked_return_type = "TrustRecord | undefined")]
    pub fn record(&self, target_user_id: &str) -> Result<JsValue, JsValue> {
        match self.inner.record(target_user_id) {
            Some(record) => to_js(record),
            None => Ok(JsValue::UNDEFINED),
        }
    }

    /// 記録がある相手の一覧（失効の手順 4 の候補を集める。#362）。
    #[wasm_bindgen(js_name = recordedUsers)]
    pub fn recorded_users(&self) -> Vec<String> {
        self.inner.records().map(|(user, _)| user.clone()).collect()
    }

    /// 自分のマスター鍵を信頼する（SSSS で解錠したあと）。
    #[wasm_bindgen(js_name = trustOwnMasterKey)]
    pub fn trust_own_master_key(&mut self, master: &CrossSigningKeyPair) -> Result<(), JsValue> {
        self.inner.trust_own_master_key(&master.inner).js()
    }

    /// 自分のユーザー署名鍵を履歴に記録する。
    #[wasm_bindgen(js_name = recordOwnUserSigningKey)]
    pub fn record_own_user_signing_key(
        &mut self,
        user_signing: &CrossSigningKeyPair,
    ) -> Result<(), JsValue> {
        self.inner
            .record_own_user_signing_key(&user_signing.inner)
            .js()
    }

    /// 利用者が相手の新しいマスター鍵を受け入れた（#362）。`{pin, signature}` を返す。
    #[wasm_bindgen(js_name = acceptMasterKey, unchecked_return_type = "AcceptedMasterKey")]
    pub fn accept_master_key(
        &mut self,
        user_signing: &CrossSigningKeyPair,
        target_user_id: &str,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] master_key: JsValue,
    ) -> Result<JsValue, JsValue> {
        let master_key: Value = from_js(&master_key, "masterKey")?;
        let accepted = self
            .inner
            .accept_master_key(&user_signing.inner, target_user_id, &master_key)
            .js()?;
        accepted_js(&accepted)
    }

    /// 受け入れ（#362）か SAS（#368）のピンと署名をサーバーに保存できた。記録の登録待ちを外す。
    #[wasm_bindgen(js_name = confirmAcceptedMasterKey)]
    pub fn confirm_accepted_master_key(&mut self, target_user_id: &str, master_key: &str) {
        self.inner
            .confirm_accepted_master_key(target_user_id, master_key);
    }

    /// TOFU のピンをサーバーに置けた。
    #[wasm_bindgen(js_name = confirmTofuPin)]
    pub fn confirm_tofu_pin(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "NewTofuPin")] pin: JsValue,
    ) -> Result<(), JsValue> {
        let pin: NewTofuPinJson = from_js(&pin, "pin")?;
        self.inner.confirm_tofu_pin(&pin.into());
        Ok(())
    }

    /// TOFU のピンを置けなかった（409 `E2EE_PIN_CONFLICT`）。
    #[wasm_bindgen(js_name = discardTofuPin)]
    pub fn discard_tofu_pin(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "NewTofuPin")] pin: JsValue,
    ) -> Result<(), JsValue> {
        let pin: NewTofuPinJson = from_js(&pin, "pin")?;
        self.inner.discard_tofu_pin(&pin.into());
        Ok(())
    }

    /// 配布先を判定する（規則 1〜6）。記録が変わるので、呼んだあとは保存し直す。
    ///
    /// `userSigning` は暗号ストアにある自分のユーザー署名鍵。解錠前でないときは
    /// [`evaluateWithUserSigning`](Self::evaluate_with_user_signing) を使う。
    /// `devices` は [`DeviceKeys`] の配列、それ以外は JSON の形
    /// （`users`、`new_pins`、`warnings`）で返す。
    #[wasm_bindgen(unchecked_return_type = "Distribution")]
    pub fn evaluate(
        &mut self,
        snapshot: &KeySnapshot,
        own_device_id: &str,
    ) -> Result<JsValue, JsValue> {
        distribution_to_js(self.inner.evaluate(&snapshot.inner, own_device_id, None))
    }

    /// 自分のユーザー署名鍵を使って配布先を判定する（初めて見た相手の TOFU のピンを作れる）。
    #[wasm_bindgen(js_name = evaluateWithUserSigning, unchecked_return_type = "Distribution")]
    pub fn evaluate_with_user_signing(
        &mut self,
        snapshot: &KeySnapshot,
        own_device_id: &str,
        user_signing: &CrossSigningKeyPair,
    ) -> Result<JsValue, JsValue> {
        distribution_to_js(self.inner.evaluate(
            &snapshot.inner,
            own_device_id,
            Some(&user_signing.inner),
        ))
    }

    /// 受信時の検証の手順 4 に渡す、送信ユーザーの信頼した自己署名鍵 `{self_signing_key, signature}`
    /// （`DecryptedToDevice.verifyRoomKey` の `trust`）。信頼できなければ `null`。
    /// `snapshot` は送信ユーザーと自分の `keys/query`。記録が変わることがあるので、呼んだあとは保存し直す。
    #[wasm_bindgen(js_name = senderTrust, unchecked_return_type = "SenderTrust | null")]
    pub fn sender_trust(
        &mut self,
        snapshot: &KeySnapshot,
        sender: &str,
        sender_device: &str,
    ) -> Result<JsValue, JsValue> {
        match self
            .inner
            .sender_trust(&snapshot.inner, sender, sender_device)
        {
            Some(trust) => to_js(&json!({
                "self_signing_key": trust.self_signing_key.to_base64(),
                "signature": trust.signature,
            })),
            None => Ok(JsValue::NULL),
        }
    }
}

fn distribution_to_js(distribution: core::Distribution) -> Result<JsValue, JsValue> {
    let users: Map<String, Value> = distribution
        .users
        .into_iter()
        .map(|(user_id, trust)| {
            let excluded: Map<String, Value> = trust
                .excluded_devices
                .into_iter()
                .map(|(device_id, e)| (device_id, device_exclusion(e).into()))
                .collect();
            (
                user_id,
                json!({"state": user_state(trust.state), "excluded_devices": excluded}),
            )
        })
        .collect();
    let new_pins: Vec<Value> = distribution
        .new_pins
        .into_iter()
        .map(|p| {
            json!({"target_user_id": p.target_user_id, "master_key": p.master_key, "pin": p.pin})
        })
        .collect();
    let warnings: Vec<&str> = distribution
        .warnings
        .into_iter()
        .map(trust_warning)
        .collect();
    let out = to_js(&json!({"users": users, "new_pins": new_pins, "warnings": warnings}))?;
    let devices = Array::new();
    for device in distribution.devices {
        devices.push(&DeviceKeys { inner: device }.into());
    }
    set(&out, "devices", &devices)?;
    Ok(out)
}

/// [`core::AcceptedMasterKey`] を `{target_user_id, master_key, pin, signature}` にする。
pub(crate) fn accepted_js(accepted: &core::AcceptedMasterKey) -> Result<JsValue, JsValue> {
    to_js(&json!({
        "target_user_id": accepted.target_user_id,
        "master_key": accepted.master_key,
        "pin": accepted.pin,
        "signature": accepted.signature,
    }))
}
