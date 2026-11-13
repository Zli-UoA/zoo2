//! 端末アカウントと Olm（#348、#349）。

use std::sync::{Arc, Mutex};

use zoo_crypto as core;
use zoo_crypto::Ed25519PublicKey;

use crate::{OutboundSession, Result, ZooCryptoError, bytes32, lock, parse_json};

/// 端末アカウント（[`core::DeviceAccount`]）。
///
/// ロックの順序：ほかのオブジェクトと一緒にロックするときは、いつも端末アカウントを最初に取る
/// （`OutboundSession`、`OlmSessionStore`、`InboundSessionStore` の順）。
#[derive(uniffi::Object)]
pub struct DeviceAccount {
    pub(crate) inner: Mutex<core::DeviceAccount>,
}

#[uniffi::export]
impl DeviceAccount {
    /// 新しい端末の鍵を作る。`user_id` と `device_id` は小文字のハイフン区切りの UUID
    /// （`device_id` は UUID v4）。
    #[uniffi::constructor]
    pub fn new(user_id: String, device_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::DeviceAccount::new(&user_id, &device_id)?),
        }))
    }

    /// [`pickle`](Self::pickle) で保存した端末を戻す。`storage_key` は 32 バイトの保存鍵、
    /// `user_id` と `device_id` は今ログインしているユーザーと、そのユーザーで登録した端末。
    #[uniffi::constructor]
    pub fn from_pickle(
        pickle: String,
        storage_key: Vec<u8>,
        user_id: String,
        device_id: String,
    ) -> Result<Arc<Self>> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        Ok(Arc::new(Self {
            inner: Mutex::new(core::DeviceAccount::from_pickle(
                &pickle,
                &storage_key,
                &user_id,
                &device_id,
            )?),
        }))
    }

    /// 32 バイトの保存鍵で暗号化した保存形式（JSON）。保存のたびに出力は変わる。
    pub fn pickle(&self, storage_key: Vec<u8>) -> Result<String> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        Ok(lock(&self.inner).pickle(&storage_key))
    }

    pub fn user_id(&self) -> String {
        lock(&self.inner).user_id().to_owned()
    }

    pub fn device_id(&self) -> String {
        lock(&self.inner).device_id().to_owned()
    }

    /// 識別鍵（Curve25519、パディングなしの base64）。
    pub fn curve25519_key(&self) -> String {
        lock(&self.inner).identity_keys().curve25519.to_base64()
    }

    /// 署名鍵（Ed25519、パディングなしの base64）。
    pub fn ed25519_key(&self) -> String {
        lock(&self.inner).identity_keys().ed25519.to_base64()
    }

    /// 署名鍵の鍵 ID（`ed25519:<device_id>`）。
    pub fn signing_key_id(&self) -> String {
        lock(&self.inner).signing_key_id()
    }

    /// JSON オブジェクトにこの端末の署名を足して返す。
    pub fn sign_json(&self, json: String) -> Result<String> {
        let mut value = parse_json("json", &json)?;
        lock(&self.inner).sign_json(&mut value)?;
        Ok(value.to_string())
    }

    /// 署名付きの端末鍵（`POST /api/e2ee/keys/upload` の `device_keys`）。
    pub fn device_keys(&self) -> String {
        lock(&self.inner).device_keys().to_string()
    }

    /// サーバーに置くワンタイム鍵の数の上限。
    pub fn max_number_of_one_time_keys(&self) -> u64 {
        lock(&self.inner).max_number_of_one_time_keys() as u64
    }

    /// ワンタイム鍵を `count` 個作る。
    pub fn generate_one_time_keys(&self, count: u64) {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        lock(&self.inner).generate_one_time_keys(count);
    }

    /// フォールバック鍵を作る。
    pub fn generate_fallback_key(&self) {
        lock(&self.inner).generate_fallback_key();
    }

    /// まだ公開していないワンタイム鍵（`one_time_keys` の JSON オブジェクト）。
    pub fn one_time_keys(&self) -> String {
        serde_json::Value::Object(lock(&self.inner).one_time_keys()).to_string()
    }

    /// まだ公開していないフォールバック鍵（`fallback_keys` の JSON オブジェクト）。
    pub fn fallback_keys(&self) -> String {
        serde_json::Value::Object(lock(&self.inner).fallback_keys()).to_string()
    }

    /// アップロードできた鍵を公開済みにする。
    pub fn mark_keys_as_published(&self) {
        lock(&self.inner).mark_keys_as_published();
    }

    /// `keys/claim` の相手のワンタイム鍵（`signed_curve25519:<key_id>` の値の JSON）で Olm セッションを作り、
    /// `sessions` に足す。返すのはセッション ID。
    pub fn create_outbound_olm_session(
        &self,
        sessions: &OlmSessionStore,
        recipient: &VerifiedDeviceKeys,
        one_time_key: String,
    ) -> Result<String> {
        let one_time_key = parse_json("one_time_key", &one_time_key)?;
        let session =
            lock(&self.inner).create_outbound_olm_session(&recipient.inner, &one_time_key)?;
        let session_id = session.session_id();
        lock(&sessions.inner).push(session);
        Ok(session_id)
    }

    /// `outbound` の Megolm セッション鍵（今のインデックスから）を `recipient` に送る to-device の
    /// `content`（JSON）を作る。`session_id` は `sessions` にある、`recipient` との Olm セッション。
    ///
    /// セッション鍵は秘密なので Kotlin に出さず、ここで `outbound` から作る（UniFFI の Record は
    /// Kotlin の `data class` になり、`toString()` に中身が出るため）。ロックは端末アカウント、
    /// `outbound`、`sessions` の順に取る。
    pub fn encrypt_room_key(
        &self,
        sessions: &OlmSessionStore,
        session_id: String,
        recipient: &VerifiedDeviceKeys,
        outbound: &OutboundSession,
    ) -> Result<String> {
        let account = lock(&self.inner);
        let room_key = lock(&outbound.inner).room_key();
        let mut sessions = lock(&sessions.inner);
        let session = sessions
            .iter_mut()
            .find(|s| s.session_id() == session_id)
            .ok_or_else(|| ZooCryptoError::Olm {
                reason: format!("Olm セッション {session_id} がありません"),
            })?;
        let content = account.encrypt_room_key(session, &recipient.inner, &room_key)?;
        Ok(content.to_string())
    }

    /// 受け取った to-device の `content`（JSON）を復号する。pre-key メッセージで新しいセッションを
    /// 作ったら `sessions` に足し、ワンタイム鍵を 1 つ使う。成功したら、呼び出し側は端末アカウントと
    /// `sessions` の両方を保存し直す。
    ///
    /// 返したものはまだ検証していない。[`OlmDecrypted::verify_room_key`] で確かめてから使う。
    pub fn decrypt_to_device(
        &self,
        sessions: &OlmSessionStore,
        content: String,
    ) -> Result<Arc<OlmDecrypted>> {
        let content = parse_json("content", &content)?;
        let mut account = lock(&self.inner);
        let mut sessions = lock(&sessions.inner);
        let decrypted = account.decrypt_to_device(&mut sessions, &content)?;
        Ok(Arc::new(OlmDecrypted { inner: decrypted }))
    }
}

/// この端末が持つ Olm セッションの一覧。
///
/// コアの Olm セッションは、まだ保存形式を持たない（暗号ストアは #359）。
#[derive(uniffi::Object, Default)]
pub struct OlmSessionStore {
    inner: Mutex<Vec<core::OlmSession>>,
}

#[uniffi::export]
impl OlmSessionStore {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// セッションの数。
    pub fn len(&self) -> u64 {
        lock(&self.inner).len() as u64
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.inner).is_empty()
    }

    /// 相手の識別鍵（base64）ごとのセッション ID。
    pub fn sessions(&self) -> Vec<OlmSessionInfo> {
        lock(&self.inner)
            .iter()
            .map(|s| OlmSessionInfo {
                session_id: s.session_id(),
                their_identity_key: s.their_identity_key().to_base64(),
            })
            .collect()
    }
}

/// Olm セッション 1 つの識別子。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OlmSessionInfo {
    pub session_id: String,
    /// 相手の端末の識別鍵（パディングなしの base64）。
    pub their_identity_key: String,
}

/// 検証済みの、他の端末の端末鍵（[`core::DeviceKeys`]）。
#[derive(uniffi::Object)]
pub struct VerifiedDeviceKeys {
    pub(crate) inner: core::DeviceKeys,
}

#[uniffi::export]
impl VerifiedDeviceKeys {
    pub fn user_id(&self) -> String {
        self.inner.user_id.clone()
    }

    pub fn device_id(&self) -> String {
        self.inner.device_id.clone()
    }

    pub fn algorithms(&self) -> Vec<String> {
        self.inner.algorithms.clone()
    }

    pub fn curve25519_key(&self) -> String {
        self.inner.curve25519.to_base64()
    }

    pub fn ed25519_key(&self) -> String {
        self.inner.ed25519.to_base64()
    }
}

/// サーバーから受け取った端末鍵の JSON を検証する（自己署名と、`user_id` と `device_id` の一致）。
#[uniffi::export]
pub fn verify_device_keys(
    device_keys: String,
    user_id: String,
    device_id: String,
) -> Result<Arc<VerifiedDeviceKeys>> {
    let value = parse_json("device_keys", &device_keys)?;
    Ok(Arc::new(VerifiedDeviceKeys {
        inner: core::verify_device_keys(&value, &user_id, &device_id)?,
    }))
}

/// 署名付き JSON の `user_id` の `key_id` の署名を `public_key`（Ed25519、base64）で検証する。
#[uniffi::export]
pub fn verify_json_signature(
    json: String,
    user_id: String,
    key_id: String,
    public_key: String,
) -> Result<()> {
    let value = parse_json("json", &json)?;
    let public_key = ed25519_from_base64(&public_key)?;
    Ok(core::verify_json_signature(
        &value,
        &user_id,
        &key_id,
        &public_key,
    )?)
}

/// Matrix と同じ Canonical JSON にする。
#[uniffi::export]
pub fn to_canonical_json(json: String) -> Result<String> {
    Ok(core::to_canonical_json(&parse_json("json", &json)?)?)
}

/// 送信端末を検証済みにするための、送信ユーザーのクロス署名の情報（[`core::SenderTrust`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SenderTrust {
    /// 信頼したマスター鍵で署名された自己署名鍵（base64）。
    pub self_signing_key: String,
    /// `e2ee_key_signatures` にある、自己署名鍵による端末鍵への署名（base64）。
    pub signature: Option<String>,
}

/// [`DeviceAccount::decrypt_to_device`] で復号した、まだ検証していない平文。
#[derive(uniffi::Object)]
pub struct OlmDecrypted {
    inner: core::OlmDecrypted,
}

#[uniffi::export]
impl OlmDecrypted {
    /// 送信端末の識別鍵（base64）。
    pub fn sender_key(&self) -> String {
        self.inner.sender_key().to_base64()
    }

    /// 平文の `sender`。
    pub fn sender(&self) -> Option<String> {
        self.inner.sender().map(str::to_owned)
    }

    /// 平文の `sender_device`。
    pub fn sender_device(&self) -> Option<String> {
        self.inner.sender_device().map(str::to_owned)
    }

    /// 平文の `type`。
    pub fn event_type(&self) -> Option<String> {
        self.inner
            .plaintext()
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    /// `zoo.room_key` を受信時の検証の手順 1〜4 で確かめる。
    ///
    /// - `listed_device_keys` は、平文の `sender` と `sender_device` で探した Shape の行か
    ///   `keys/query` の応答の `device_keys`（JSON）。どちらにもなければ `null`。
    /// - `trust` は送信ユーザーの自己署名鍵。クロス署名を確かめられないときは `null`。
    pub fn verify_room_key(
        &self,
        account: &DeviceAccount,
        listed_device_keys: Option<String>,
        trust: Option<SenderTrust>,
    ) -> Result<Arc<ReceivedRoomKey>> {
        let listed = listed_device_keys
            .as_deref()
            .map(|json| parse_json("listed_device_keys", json))
            .transpose()?;
        let listed = match &listed {
            Some(value) => core::ListedDevice::Listed(value),
            None => core::ListedDevice::NotListed,
        };
        let trust = trust
            .map(|t| -> Result<core::SenderTrust> {
                Ok(core::SenderTrust {
                    self_signing_key: ed25519_from_base64(&t.self_signing_key)?,
                    signature: t.signature,
                })
            })
            .transpose()?;
        let account = lock(&account.inner);
        let key = self
            .inner
            .verify_room_key(&account, listed, trust.as_ref())?;
        Ok(Arc::new(ReceivedRoomKey::new(key)))
    }
}

/// 検証を通った Megolm セッション鍵。[`InboundSessionStore::add_room_key`](crate::InboundSessionStore::add_room_key)
/// で一度だけ保存できる。
#[derive(uniffi::Object)]
pub struct ReceivedRoomKey {
    pub(crate) inner: Mutex<Option<core::ReceivedRoomKey>>,
    channel_id: String,
    epoch: u64,
    session_id: String,
    sender_key: String,
    sender_user_id: String,
    sender_device_id: String,
    sender_verified: bool,
}

impl ReceivedRoomKey {
    fn new(key: core::ReceivedRoomKey) -> Self {
        Self {
            channel_id: key.channel_id().to_owned(),
            epoch: key.epoch(),
            session_id: key.session_id(),
            sender_key: key.sender_key().to_base64(),
            sender_user_id: key.sender_user_id().to_owned(),
            sender_device_id: key.sender_device_id().to_owned(),
            sender_verified: key.sender_verified(),
            inner: Mutex::new(Some(key)),
        }
    }
}

#[uniffi::export]
impl ReceivedRoomKey {
    pub fn channel_id(&self) -> String {
        self.channel_id.clone()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn session_id(&self) -> String {
        self.session_id.clone()
    }

    pub fn sender_key(&self) -> String {
        self.sender_key.clone()
    }

    pub fn sender_user_id(&self) -> String {
        self.sender_user_id.clone()
    }

    pub fn sender_device_id(&self) -> String {
        self.sender_device_id.clone()
    }

    /// 送信端末を検証済みにできたか。`false` なら「未検証の送信者」と表示する。
    pub fn sender_verified(&self) -> bool {
        self.sender_verified
    }
}

pub(crate) fn ed25519_from_base64(key: &str) -> Result<Ed25519PublicKey> {
    Ed25519PublicKey::from_base64(key).map_err(|e| core::CryptoError::from(e).into())
}
