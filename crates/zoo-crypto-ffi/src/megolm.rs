//! Megolm の送信用セッションと受信用セッション（#349）。

use std::sync::{Arc, Mutex};

use zoo_crypto as core;

use crate::{DeviceAccount, ReceivedRoomKey, Result, ZooCryptoError, lock, parse_json};

/// チャンネルごとの Megolm の送信用セッション（[`core::OutboundSession`]）。
///
/// コアの送信用セッションは、まだ保存形式を持たない（暗号ストアは #359）。
#[derive(uniffi::Object)]
pub struct OutboundSession {
    pub(crate) inner: Mutex<core::OutboundSession>,
}

#[uniffi::export]
impl OutboundSession {
    /// `channel_id` の、`epoch`（`membership_epoch`）の送信用セッションを作る。`now_ms` は今の時刻（Unix ミリ秒）。
    #[uniffi::constructor]
    pub fn new(channel_id: String, epoch: u64, now_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(core::OutboundSession::new(&channel_id, epoch, now_ms)),
        })
    }

    pub fn session_id(&self) -> String {
        lock(&self.inner).session_id()
    }

    pub fn channel_id(&self) -> String {
        lock(&self.inner).channel_id().to_owned()
    }

    pub fn epoch(&self) -> u64 {
        lock(&self.inner).epoch()
    }

    /// このセッションで暗号化したメッセージの数。
    pub fn message_count(&self) -> u64 {
        lock(&self.inner).message_count()
    }

    /// 次に暗号化するメッセージのインデックス。
    pub fn message_index(&self) -> u32 {
        lock(&self.inner).message_index()
    }

    /// 作り直すべきか（期間、メッセージ数、`membership_epoch` の変化）。
    pub fn needs_rotation(&self, now_ms: u64, current_epoch: u64) -> bool {
        lock(&self.inner).needs_rotation(now_ms, current_epoch)
    }

    /// メッセージを暗号化し、`messages` の `ciphertext`（JSON）を返す。
    pub fn encrypt(&self, account: &DeviceAccount, message: OutgoingMessage) -> Result<String> {
        let account = lock(&account.inner);
        let value = lock(&self.inner).encrypt(&account, &message.into())?;
        Ok(value.to_string())
    }
}

/// Megolm の受信用セッションと、リプレイと版の記録（[`core::InboundSessionStore`]）。
///
/// コアの受信用セッションは、まだ保存形式を持たない（暗号ストアは #359）。
#[derive(uniffi::Object, Default)]
pub struct InboundSessionStore {
    pub(crate) inner: Mutex<core::InboundSessionStore>,
}

#[uniffi::export]
impl InboundSessionStore {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 検証を通ったセッション鍵を保存する。同じ `room_key` は一度しか使えない。
    pub fn add_room_key(&self, room_key: &ReceivedRoomKey) -> Result<AddOutcome> {
        let key = lock(&room_key.inner)
            .take()
            .ok_or_else(|| ZooCryptoError::AlreadyConsumed {
                reason: "この ReceivedRoomKey は保存済みです".into(),
            })?;
        Ok(lock(&self.inner).add_room_key(key).into())
    }

    /// 自分の送信用セッションを、自分のメッセージを読むために受信用セッションとして保存する。
    pub fn add_outbound(&self, account: &DeviceAccount, outbound: &OutboundSession) -> AddOutcome {
        let account = lock(&account.inner);
        let outbound = lock(&outbound.inner);
        lock(&self.inner).add_outbound(&account, &outbound).into()
    }

    /// 保存しているセッションのキー。
    pub fn session_refs(&self) -> Vec<SessionRef> {
        lock(&self.inner)
            .session_refs()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// `messages` の行を復号し、受信時の検証を行う。
    pub fn decrypt(&self, message: EncryptedMessage) -> Result<DecryptedMessage> {
        let ciphertext = parse_json("ciphertext", &message.ciphertext)?;
        let row = core::EncryptedMessage {
            channel_id: &message.channel_id,
            message_id: &message.message_id,
            parent_id: message.parent_id.as_deref(),
            user_id: &message.user_id,
            message_kind: &message.message_kind,
            epoch: message.epoch,
            revision: message.revision,
            ciphertext: &ciphertext,
        };
        Ok(lock(&self.inner).decrypt(&row)?.into())
    }
}

/// 受信用セッションのキー（[`core::SessionRef`]）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SessionRef {
    pub channel_id: String,
    /// 送信端末の識別鍵（base64）。
    pub sender_key: String,
    pub session_id: String,
}

impl From<core::SessionRef> for SessionRef {
    fn from(r: core::SessionRef) -> Self {
        Self {
            channel_id: r.channel_id,
            sender_key: r.sender_key,
            session_id: r.session_id,
        }
    }
}

impl From<SessionRef> for core::SessionRef {
    fn from(r: SessionRef) -> Self {
        Self {
            channel_id: r.channel_id,
            sender_key: r.sender_key,
            session_id: r.session_id,
        }
    }
}

/// 受信用セッションを保存しようとした結果（[`core::AddOutcome`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AddOutcome {
    Added,
    Replaced,
    Kept,
}

impl From<core::AddOutcome> for AddOutcome {
    fn from(outcome: core::AddOutcome) -> Self {
        match outcome {
            core::AddOutcome::Added => Self::Added,
            core::AddOutcome::Replaced => Self::Replaced,
            core::AddOutcome::Kept => Self::Kept,
        }
    }
}

/// 本文（[`core::MessageContent`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageContent {
    /// 本文（MFM）。
    pub body: String,
    /// `mfm` だけ。
    pub format: String,
    /// `message` か `emote`。
    pub kind: String,
}

/// メンション（[`core::Mentions`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct Mentions {
    pub user_ids: Vec<String>,
    pub channel: bool,
    pub here: bool,
}

/// 送るメッセージ（[`core::OutgoingMessage`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutgoingMessage {
    pub message_id: String,
    pub parent_id: Option<String>,
    /// 新規送信は 0、編集のたびに 1 増やす。
    pub revision: u64,
    pub content: MessageContent,
    pub mentions: Mentions,
}

/// 復号する `messages` の行（[`core::EncryptedMessage`]）。`ciphertext` は行の JSON のまま。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EncryptedMessage {
    pub channel_id: String,
    pub message_id: String,
    pub parent_id: Option<String>,
    pub user_id: String,
    pub message_kind: String,
    pub epoch: u64,
    pub revision: u64,
    pub ciphertext: String,
}

/// 復号した平文（[`core::MessagePayload`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessagePayload {
    pub payload_type: String,
    pub channel_id: String,
    pub message_id: String,
    pub parent_id: Option<String>,
    pub epoch: u64,
    pub revision: u64,
    pub content: MessageContent,
    pub mentions: Mentions,
}

/// 受信用セッションの入手経路（[`core::SessionOrigin`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SessionOrigin {
    Olm,
    /// 鍵バックアップから取り込んだ。「送信者を確かめられない」の印を付ける。
    Backup,
}

/// 版の照合の結果（[`core::RevisionStatus`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RevisionStatus {
    Latest,
    Stale { latest: u64 },
    Conflict,
}

/// 検証を通ったメッセージ（[`core::DecryptedMessage`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DecryptedMessage {
    pub payload: MessagePayload,
    pub message_index: u32,
    pub origin: SessionOrigin,
    pub sender_verified: bool,
    pub revision: RevisionStatus,
}

impl From<MessageContent> for core::MessageContent {
    fn from(c: MessageContent) -> Self {
        Self {
            body: c.body,
            format: c.format,
            kind: c.kind,
        }
    }
}

impl From<core::MessageContent> for MessageContent {
    fn from(c: core::MessageContent) -> Self {
        Self {
            body: c.body,
            format: c.format,
            kind: c.kind,
        }
    }
}

impl From<Mentions> for core::Mentions {
    fn from(m: Mentions) -> Self {
        Self {
            user_ids: m.user_ids,
            channel: m.channel,
            here: m.here,
        }
    }
}

impl From<core::Mentions> for Mentions {
    fn from(m: core::Mentions) -> Self {
        Self {
            user_ids: m.user_ids,
            channel: m.channel,
            here: m.here,
        }
    }
}

impl From<OutgoingMessage> for core::OutgoingMessage {
    fn from(m: OutgoingMessage) -> Self {
        Self {
            message_id: m.message_id,
            parent_id: m.parent_id,
            revision: m.revision,
            content: m.content.into(),
            mentions: m.mentions.into(),
        }
    }
}

impl From<core::DecryptedMessage> for DecryptedMessage {
    fn from(m: core::DecryptedMessage) -> Self {
        let p = m.payload;
        Self {
            payload: MessagePayload {
                payload_type: p.payload_type,
                channel_id: p.channel_id,
                message_id: p.message_id,
                parent_id: p.parent_id,
                epoch: p.epoch,
                revision: p.revision,
                content: p.content.into(),
                mentions: p.mentions.into(),
            },
            message_index: m.message_index,
            origin: match m.origin {
                core::SessionOrigin::Olm => SessionOrigin::Olm,
                core::SessionOrigin::Backup => SessionOrigin::Backup,
            },
            sender_verified: m.sender_verified,
            revision: match m.revision {
                core::RevisionStatus::Latest => RevisionStatus::Latest,
                core::RevisionStatus::Stale { latest } => RevisionStatus::Stale { latest },
                core::RevisionStatus::Conflict => RevisionStatus::Conflict,
            },
        }
    }
}
