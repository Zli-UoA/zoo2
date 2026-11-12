//! 端末アカウント、端末鍵の検証、Olm（#348、#349）。

use serde_json::Value;
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::convert::{IntoJs, bytes32, ed25519, from_js, from_js_opt, to_js};
use crate::megolm::RoomKey;

/// 端末の識別鍵、ワンタイム鍵、フォールバック鍵（`zoo_crypto::DeviceAccount`）。
///
/// 保存は [`pickle`](Self::pickle) で、呼び出し側の 32 バイトの保存鍵から保存ごとに導いた鍵で
/// 暗号化する。ワンタイム鍵を作る、使う（Olm のセッションを受ける）たびに保存し直す。
#[wasm_bindgen]
pub struct DeviceAccount {
    pub(crate) inner: core::DeviceAccount,
}

#[wasm_bindgen]
impl DeviceAccount {
    /// 新しい端末を作る。`userId` と `deviceId` は小文字でハイフン区切りの UUID
    /// （`deviceId` は UUID v4）。
    #[wasm_bindgen(constructor)]
    pub fn new(user_id: &str, device_id: &str) -> Result<DeviceAccount, JsValue> {
        Ok(Self {
            inner: core::DeviceAccount::new(user_id, device_id).js()?,
        })
    }

    /// [`pickle`](Self::pickle) で保存した状態を、同じ保存鍵で復元する。持ち主が
    /// `userId` と `deviceId` でなければ `PickleOwnerMismatch` で失敗する。
    #[wasm_bindgen(js_name = fromPickle)]
    pub fn from_pickle(
        pickle: &str,
        storage_key: &[u8],
        user_id: &str,
        device_id: &str,
    ) -> Result<DeviceAccount, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(Self {
            inner: core::DeviceAccount::from_pickle(pickle, &key, user_id, device_id).js()?,
        })
    }

    /// 保存する文字列（JSON）を作る。`storageKey` は 32 バイト。
    pub fn pickle(&self, storage_key: &[u8]) -> Result<String, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        Ok(self.inner.pickle(&key))
    }

    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> String {
        self.inner.user_id().to_owned()
    }

    #[wasm_bindgen(getter, js_name = deviceId)]
    pub fn device_id(&self) -> String {
        self.inner.device_id().to_owned()
    }

    /// 識別鍵 `{curve25519, ed25519}`（パディングなしの base64）。
    #[wasm_bindgen(js_name = identityKeys, unchecked_return_type = "IdentityKeys")]
    pub fn identity_keys(&self) -> Result<JsValue, JsValue> {
        let keys = self.inner.identity_keys();
        to_js(&serde_json::json!({
            "curve25519": keys.curve25519.to_base64(),
            "ed25519": keys.ed25519.to_base64(),
        }))
    }

    /// 端末の署名鍵の ID（`ed25519:<deviceId>`）。
    #[wasm_bindgen(getter, js_name = signingKeyId)]
    pub fn signing_key_id(&self) -> String {
        self.inner.signing_key_id()
    }

    /// `value` に端末の署名鍵で署名し、`signatures` を足した JSON を返す。
    #[wasm_bindgen(js_name = signJson, unchecked_return_type = "JsonObject")]
    pub fn sign_json(
        &self,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
    ) -> Result<JsValue, JsValue> {
        let mut value: Value = from_js(&value, "value")?;
        self.inner.sign_json(&mut value).js()?;
        to_js(&value)
    }

    /// 自己署名した端末鍵（`device_keys`）。
    #[wasm_bindgen(js_name = deviceKeys, unchecked_return_type = "JsonObject")]
    pub fn device_keys(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.device_keys())
    }

    /// サーバーに置いておくワンタイム鍵の数（docs/e2ee.md の【提案】の 50 個）。
    #[wasm_bindgen(getter, js_name = maxNumberOfOneTimeKeys)]
    pub fn max_number_of_one_time_keys(&self) -> usize {
        self.inner.max_number_of_one_time_keys()
    }

    /// ワンタイム鍵を `count` 個作る。
    #[wasm_bindgen(js_name = generateOneTimeKeys)]
    pub fn generate_one_time_keys(&mut self, count: usize) {
        self.inner.generate_one_time_keys(count);
    }

    /// フォールバック鍵を作り直す。
    #[wasm_bindgen(js_name = generateFallbackKey)]
    pub fn generate_fallback_key(&mut self) {
        self.inner.generate_fallback_key();
    }

    /// まだ公開していない、署名付きのワンタイム鍵（`signed_curve25519:<key_id>` をキーにしたオブジェクト）。
    #[wasm_bindgen(js_name = oneTimeKeys, unchecked_return_type = "Record<string, JsonObject>")]
    pub fn one_time_keys(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.one_time_keys())
    }

    /// まだ公開していない、署名付きのフォールバック鍵。
    #[wasm_bindgen(js_name = fallbackKeys, unchecked_return_type = "Record<string, JsonObject>")]
    pub fn fallback_keys(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.fallback_keys())
    }

    /// アップロードできた鍵を公開済みにする。
    #[wasm_bindgen(js_name = markKeysAsPublished)]
    pub fn mark_keys_as_published(&mut self) {
        self.inner.mark_keys_as_published();
    }

    /// `keys/claim` で受け取った相手のワンタイム鍵（`{key, signatures, fallback?}`）で、
    /// Olm のセッションを作る。ワンタイム鍵の署名は `recipient` の署名鍵で検証する。
    #[wasm_bindgen(js_name = createOutboundOlmSession)]
    pub fn create_outbound_olm_session(
        &self,
        recipient: &DeviceKeys,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] one_time_key: JsValue,
    ) -> Result<OlmSession, JsValue> {
        let one_time_key: Value = from_js(&one_time_key, "oneTimeKey")?;
        Ok(OlmSession {
            inner: self
                .inner
                .create_outbound_olm_session(&recipient.inner, &one_time_key)
                .js()?,
        })
    }

    /// `roomKey` を `recipient` に送る to-device メッセージの `content` を作る。
    /// `session` は `recipient` の識別鍵とのセッション。
    #[wasm_bindgen(js_name = encryptRoomKey, unchecked_return_type = "JsonObject")]
    pub fn encrypt_room_key(
        &self,
        session: &mut OlmSession,
        recipient: &DeviceKeys,
        room_key: &RoomKey,
    ) -> Result<JsValue, JsValue> {
        let content = self
            .inner
            .encrypt_room_key(&mut session.inner, &recipient.inner, &room_key.inner)
            .js()?;
        to_js(&content)
    }

    /// to-device メッセージの `content` を復号する。新しいセッションを作ったら `sessions` に足し、
    /// ワンタイム鍵を 1 つ使う。成功したら、端末アカウントを保存し直す。
    ///
    /// 返した値はまだ検証していない。[`OlmDecrypted::verify_room_key`] で検証してから使う。
    #[wasm_bindgen(js_name = decryptToDevice)]
    pub fn decrypt_to_device(
        &mut self,
        sessions: &mut OlmSessions,
        #[wasm_bindgen(unchecked_param_type = "JsonObject")] content: JsValue,
    ) -> Result<OlmDecrypted, JsValue> {
        let content: Value = from_js(&content, "content")?;
        Ok(OlmDecrypted {
            inner: self
                .inner
                .decrypt_to_device(&mut sessions.inner, &content)
                .js()?,
        })
    }
}

/// 検証済みの他の端末の端末鍵（`zoo_crypto::DeviceKeys`）。
///
/// [`verifyDeviceKeys`](verify_device_keys) か、配布先の判定（`TrustStore.evaluate`）だけが作る。
#[wasm_bindgen]
#[derive(Clone)]
pub struct DeviceKeys {
    pub(crate) inner: core::DeviceKeys,
}

#[wasm_bindgen]
impl DeviceKeys {
    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> String {
        self.inner.user_id.clone()
    }

    #[wasm_bindgen(getter, js_name = deviceId)]
    pub fn device_id(&self) -> String {
        self.inner.device_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn algorithms(&self) -> Vec<String> {
        self.inner.algorithms.clone()
    }

    /// 識別鍵（パディングなしの base64）。
    #[wasm_bindgen(getter)]
    pub fn curve25519(&self) -> String {
        self.inner.curve25519.to_base64()
    }

    /// 署名鍵（パディングなしの base64）。
    #[wasm_bindgen(getter)]
    pub fn ed25519(&self) -> String {
        self.inner.ed25519.to_base64()
    }

    /// `{user_id, device_id, algorithms, curve25519, ed25519}`。`JSON.stringify` にも使う。
    #[wasm_bindgen(js_name = toJSON, unchecked_return_type = "DeviceKeysJson")]
    pub fn to_json(&self) -> Result<JsValue, JsValue> {
        to_js(&serde_json::json!({
            "user_id": self.inner.user_id,
            "device_id": self.inner.device_id,
            "algorithms": self.inner.algorithms,
            "curve25519": self.inner.curve25519.to_base64(),
            "ed25519": self.inner.ed25519.to_base64(),
        }))
    }
}

/// サーバーから受け取った端末鍵を検証する（`user_id`、`device_id`、鍵、自己署名）。
#[wasm_bindgen(js_name = verifyDeviceKeys)]
pub fn verify_device_keys(
    #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
    user_id: &str,
    device_id: &str,
) -> Result<DeviceKeys, JsValue> {
    let value: Value = from_js(&value, "value")?;
    Ok(DeviceKeys {
        inner: core::verify_device_keys(&value, user_id, device_id).js()?,
    })
}

/// Olm のセッション 1 つ。保存（暗号ストア）は #359 で足すので、今はメモリー上にだけある。
#[wasm_bindgen]
pub struct OlmSession {
    pub(crate) inner: core::OlmSession,
}

#[wasm_bindgen]
impl OlmSession {
    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    /// 相手の端末の識別鍵（パディングなしの base64）。
    #[wasm_bindgen(getter, js_name = theirIdentityKey)]
    pub fn their_identity_key(&self) -> String {
        self.inner.their_identity_key().to_base64()
    }
}

/// 受信に使う Olm のセッションの一覧（`decryptToDevice` が探し、新しいものを足す）。
#[wasm_bindgen]
#[derive(Default)]
pub struct OlmSessions {
    inner: Vec<core::OlmSession>,
}

#[wasm_bindgen]
impl OlmSessions {
    #[wasm_bindgen(constructor)]
    pub fn new() -> OlmSessions {
        Self::default()
    }

    #[wasm_bindgen(getter)]
    pub fn length(&self) -> usize {
        self.inner.len()
    }

    /// セッションを足す。渡した `session` は JS からは使えなくなる。
    pub fn push(&mut self, session: OlmSession) {
        self.inner.push(session.inner);
    }

    /// 各セッションの ID。
    #[wasm_bindgen(js_name = sessionIds)]
    pub fn session_ids(&self) -> Vec<String> {
        self.inner
            .iter()
            .map(core::OlmSession::session_id)
            .collect()
    }
}

/// 復号しただけで、まだ検証していない to-device メッセージ。
#[wasm_bindgen]
pub struct OlmDecrypted {
    inner: core::OlmDecrypted,
}

#[wasm_bindgen]
impl OlmDecrypted {
    /// 送信端末の識別鍵（Olm のセッションの相手）。
    #[wasm_bindgen(getter, js_name = senderKey)]
    pub fn sender_key(&self) -> String {
        self.inner.sender_key().to_base64()
    }

    /// 平文の `sender`（検証前）。
    #[wasm_bindgen(getter)]
    pub fn sender(&self) -> Option<String> {
        self.inner.sender().map(str::to_owned)
    }

    /// 平文の `sender_device`（検証前）。
    #[wasm_bindgen(getter, js_name = senderDevice)]
    pub fn sender_device(&self) -> Option<String> {
        self.inner.sender_device().map(str::to_owned)
    }

    /// 復号した平文（検証前）。
    #[wasm_bindgen(getter, unchecked_return_type = "JsonObject")]
    pub fn plaintext(&self) -> Result<JsValue, JsValue> {
        to_js(self.inner.plaintext())
    }

    /// `zoo.room_key` を検証する（docs/e2ee.md の「受信時の検証」の手順 1〜4）。
    ///
    /// - `listedDeviceKeys` は、平文の `sender` と `sender_device` で探した Shape の行か
    ///   `keys/query` の応答の `device_keys`。どちらにもなければ `null`。
    /// - `trust` は送信ユーザーの、信頼したマスター鍵で署名された自己署名鍵
    ///   `{self_signing_key, signature?}`。クロス署名を確かめられないときは `null`。
    #[wasm_bindgen(js_name = verifyRoomKey)]
    pub fn verify_room_key(
        &self,
        account: &DeviceAccount,
        #[wasm_bindgen(unchecked_param_type = "JsonObject | null | undefined")]
        listed_device_keys: JsValue,
        #[wasm_bindgen(unchecked_param_type = "SenderTrust | null | undefined")] trust: JsValue,
    ) -> Result<ReceivedRoomKey, JsValue> {
        let listed: Option<Value> = from_js_opt(&listed_device_keys, "listedDeviceKeys")?;
        let listed = match &listed {
            Some(value) => core::ListedDevice::Listed(value),
            None => core::ListedDevice::NotListed,
        };
        let trust: Option<SenderTrustJson> = from_js_opt(&trust, "trust")?;
        let trust = trust
            .map(|t| {
                Ok::<_, JsValue>(core::SenderTrust {
                    self_signing_key: ed25519(&t.self_signing_key)?,
                    signature: t.signature,
                })
            })
            .transpose()?;
        Ok(ReceivedRoomKey {
            inner: self
                .inner
                .verify_room_key(&account.inner, listed, trust.as_ref())
                .js()?,
        })
    }
}

#[derive(serde::Deserialize)]
struct SenderTrustJson {
    self_signing_key: String,
    #[serde(default)]
    signature: Option<String>,
}

/// 検証を通った Megolm セッション鍵。`InboundSessionStore.addRoomKey` で保存する。
#[wasm_bindgen]
pub struct ReceivedRoomKey {
    pub(crate) inner: core::ReceivedRoomKey,
}

#[wasm_bindgen]
impl ReceivedRoomKey {
    #[wasm_bindgen(getter, js_name = channelId)]
    pub fn channel_id(&self) -> String {
        self.inner.channel_id().to_owned()
    }

    #[wasm_bindgen(getter)]
    pub fn epoch(&self) -> f64 {
        self.inner.epoch() as f64
    }

    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    #[wasm_bindgen(getter, js_name = senderKey)]
    pub fn sender_key(&self) -> String {
        self.inner.sender_key().to_base64()
    }

    #[wasm_bindgen(getter, js_name = senderUserId)]
    pub fn sender_user_id(&self) -> String {
        self.inner.sender_user_id().to_owned()
    }

    #[wasm_bindgen(getter, js_name = senderDeviceId)]
    pub fn sender_device_id(&self) -> String {
        self.inner.sender_device_id().to_owned()
    }

    /// 送信端末を検証済みにできたか。`false` なら「未検証の送信者」と表示する。
    #[wasm_bindgen(getter, js_name = senderVerified)]
    pub fn sender_verified(&self) -> bool {
        self.inner.sender_verified()
    }
}
