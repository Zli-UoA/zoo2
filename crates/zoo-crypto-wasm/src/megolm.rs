//! Megolm の送信用セッションと受信用セッション（#349）。

use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

use crate::convert::{IntoJs, bytes32, from_js, safe_u64, to_js};
use crate::device::{DeviceAccount, ReceivedRoomKey};

/// チャンネルに配る Megolm セッション鍵（`zoo.room_key` の中身）。
///
/// セッション鍵そのものは JS に出さない。`DeviceAccount.encryptRoomKey` に渡して使う。
#[wasm_bindgen]
pub struct RoomKey {
    pub(crate) inner: core::RoomKey,
}

#[wasm_bindgen]
impl RoomKey {
    #[wasm_bindgen(getter, js_name = channelId)]
    pub fn channel_id(&self) -> String {
        self.inner.channel_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn epoch(&self) -> f64 {
        self.inner.epoch as f64
    }

    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.inner.session_id.clone()
    }
}

/// チャンネルごとの送信用セッション。保存（暗号ストア）は #359 で足す。
#[wasm_bindgen]
pub struct OutboundSession {
    pub(crate) inner: core::OutboundSession,
}

#[derive(Deserialize)]
struct OutgoingMessageJson {
    message_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    revision: u64,
    content: core::MessageContent,
    #[serde(default)]
    mentions: core::Mentions,
}

#[wasm_bindgen]
impl OutboundSession {
    /// `channelId` のチャンネルに、`epoch`（`membership_epoch`）で新しいセッションを作る。
    /// `nowMs` は今の時刻（Unix エポックからのミリ秒）。
    #[wasm_bindgen(constructor)]
    pub fn new(channel_id: &str, epoch: f64, now_ms: f64) -> Result<OutboundSession, JsValue> {
        Ok(Self {
            inner: core::OutboundSession::new(
                channel_id,
                safe_u64(epoch, "epoch")?,
                safe_u64(now_ms, "nowMs")?,
            ),
        })
    }

    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.inner.session_id()
    }

    #[wasm_bindgen(getter, js_name = channelId)]
    pub fn channel_id(&self) -> String {
        self.inner.channel_id().to_owned()
    }

    #[wasm_bindgen(getter)]
    pub fn epoch(&self) -> f64 {
        self.inner.epoch() as f64
    }

    /// このセッションで暗号化したメッセージの数。
    #[wasm_bindgen(getter, js_name = messageCount)]
    pub fn message_count(&self) -> f64 {
        self.inner.message_count() as f64
    }

    /// 次に暗号化するメッセージのインデックス。
    #[wasm_bindgen(getter, js_name = messageIndex)]
    pub fn message_index(&self) -> u32 {
        self.inner.message_index()
    }

    /// 今のインデックスからのセッション鍵。
    #[wasm_bindgen(js_name = roomKey)]
    pub fn room_key(&self) -> RoomKey {
        RoomKey {
            inner: self.inner.room_key(),
        }
    }

    /// 作り直すべきか（期間、メッセージ数、`membership_epoch` の変化）。
    #[wasm_bindgen(js_name = needsRotation)]
    pub fn needs_rotation(&self, now_ms: f64, current_epoch: f64) -> Result<bool, JsValue> {
        Ok(self.inner.needs_rotation(
            safe_u64(now_ms, "nowMs")?,
            safe_u64(current_epoch, "currentEpoch")?,
        ))
    }

    /// メッセージ `{message_id, parent_id?, revision, content, mentions?}` を暗号化し、
    /// `messages.ciphertext` に入れる JSON を返す。
    #[wasm_bindgen(unchecked_return_type = "JsonObject")]
    pub fn encrypt(
        &mut self,
        account: &DeviceAccount,
        #[wasm_bindgen(unchecked_param_type = "OutgoingMessage")] message: JsValue,
    ) -> Result<JsValue, JsValue> {
        let message: OutgoingMessageJson = from_js(&message, "message")?;
        let message = core::OutgoingMessage {
            message_id: message.message_id,
            parent_id: message.parent_id,
            revision: message.revision,
            content: message.content,
            mentions: message.mentions,
        };
        to_js(&self.inner.encrypt(&account.inner, &message).js()?)
    }
}

/// 鍵バックアップから取り出したセッション 1 つ。`InboundSessionStore.importBackupSession` で保存する。
#[wasm_bindgen]
pub struct BackupSession {
    pub(crate) inner: core::BackupSession,
}

#[wasm_bindgen]
impl BackupSession {
    #[wasm_bindgen(getter, js_name = channelId)]
    pub fn channel_id(&self) -> String {
        self.inner.channel_id.clone()
    }

    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.inner.session_id.clone()
    }

    #[wasm_bindgen(getter, js_name = senderKey)]
    pub fn sender_key(&self) -> String {
        self.inner.sender_key.to_base64()
    }

    #[wasm_bindgen(getter)]
    pub fn epoch(&self) -> f64 {
        self.inner.epoch as f64
    }
}

fn add_outcome(outcome: core::AddOutcome) -> &'static str {
    match outcome {
        core::AddOutcome::Added => "added",
        core::AddOutcome::Replaced => "replaced",
        core::AddOutcome::Kept => "kept",
    }
}

#[derive(Deserialize)]
struct EncryptedMessageJson {
    channel_id: String,
    /// `messages.id`。Shape の行と同じ名前の `id` も受け付ける。
    #[serde(alias = "id")]
    message_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    user_id: String,
    message_kind: String,
    epoch: u64,
    revision: u64,
    ciphertext: Value,
}

/// Megolm の受信用セッションと、リプレイと版の記録。保存（暗号ストア）は #359 で足す。
#[wasm_bindgen]
#[derive(Default)]
pub struct InboundSessionStore {
    pub(crate) inner: core::InboundSessionStore,
}

#[wasm_bindgen]
impl InboundSessionStore {
    #[wasm_bindgen(constructor)]
    pub fn new() -> InboundSessionStore {
        Self::default()
    }

    /// 検証を通ったセッション鍵を保存する。`"added"`、`"replaced"`、`"kept"` のどれかを返す。
    /// 渡した `roomKey` は JS からは使えなくなる。
    #[wasm_bindgen(js_name = addRoomKey, unchecked_return_type = "AddOutcome")]
    pub fn add_room_key(&mut self, room_key: ReceivedRoomKey) -> String {
        add_outcome(self.inner.add_room_key(room_key.inner)).to_owned()
    }

    /// 自分の送信用セッションを、自分のメッセージを読むために保存する。
    #[wasm_bindgen(js_name = addOutbound, unchecked_return_type = "AddOutcome")]
    pub fn add_outbound(&mut self, account: &DeviceAccount, outbound: &OutboundSession) -> String {
        add_outcome(self.inner.add_outbound(&account.inner, &outbound.inner)).to_owned()
    }

    /// 鍵バックアップのセッションを保存する。渡した `session` は JS からは使えなくなる。
    #[wasm_bindgen(js_name = importBackupSession, unchecked_return_type = "AddOutcome")]
    pub fn import_backup_session(&mut self, session: BackupSession) -> Result<String, JsValue> {
        Ok(add_outcome(self.inner.import_backup_session(session.inner).js()?).to_owned())
    }

    /// `ref` のセッション（Olm 由来とバックアップ由来）を、暗号ストアに置く文字列にする（#359）。
    /// そのキーのセッションがなければ `undefined`。リプレイと版の記録は入れない（#361 で足す）。
    #[wasm_bindgen(js_name = pickleSession)]
    pub fn pickle_session(
        &self,
        #[wasm_bindgen(unchecked_param_type = "SessionRef")] session_ref: JsValue,
        account: &DeviceAccount,
        storage_key: &[u8],
    ) -> Result<Option<String>, JsValue> {
        let session_ref: core::SessionRef = from_js(&session_ref, "ref")?;
        let key = bytes32(storage_key, "storageKey")?;
        Ok(self
            .inner
            .pickle_session(&session_ref, &account.inner, &key))
    }

    /// [`pickleSession`](Self::pickle_session) の文字列から、そのキーのセッションを戻す
    /// （同じキーがあれば置き換える）。戻したキーを返す。
    #[wasm_bindgen(js_name = restoreSession, unchecked_return_type = "SessionRef")]
    pub fn restore_session(
        &mut self,
        pickle: &str,
        account: &DeviceAccount,
        storage_key: &[u8],
    ) -> Result<JsValue, JsValue> {
        let key = bytes32(storage_key, "storageKey")?;
        let session_ref = self
            .inner
            .restore_session(pickle, &account.inner, &key)
            .js()?;
        to_js(&session_ref)
    }

    /// 保存しているセッションのキー `{channel_id, sender_key, session_id}` の一覧。
    #[wasm_bindgen(js_name = sessionRefs, unchecked_return_type = "SessionRef[]")]
    pub fn session_refs(&self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.session_refs())
    }

    /// `messages` の行を復号し、行と照合する（docs/e2ee.md の「受信時の検証」）。
    ///
    /// 鍵がまだないときは `code` が `MissingRoomKey` の、検証に失敗したときは `Rejected` の
    /// エラーを投げる。
    #[wasm_bindgen(unchecked_return_type = "DecryptedMessage")]
    pub fn decrypt(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "EncryptedMessage")] row: JsValue,
    ) -> Result<JsValue, JsValue> {
        let row: EncryptedMessageJson = from_js(&row, "row")?;
        let message = core::EncryptedMessage {
            channel_id: &row.channel_id,
            message_id: &row.message_id,
            parent_id: row.parent_id.as_deref(),
            user_id: &row.user_id,
            message_kind: &row.message_kind,
            epoch: row.epoch,
            revision: row.revision,
            ciphertext: &row.ciphertext,
        };
        let decrypted = self.inner.decrypt(&message).js()?;
        let revision = match decrypted.revision {
            core::RevisionStatus::Latest => json!({"status": "latest"}),
            core::RevisionStatus::Stale { latest } => json!({"status": "stale", "latest": latest}),
            core::RevisionStatus::Conflict => json!({"status": "conflict"}),
        };
        to_js(&json!({
            "payload": decrypted.payload,
            "message_index": decrypted.message_index,
            "origin": match decrypted.origin {
                core::SessionOrigin::Olm => "olm",
                core::SessionOrigin::Backup => "backup",
            },
            "sender_verified": decrypted.sender_verified,
            "revision": revision,
        }))
    }
}
