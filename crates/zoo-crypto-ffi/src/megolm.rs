//! Megolm の送信用セッションと受信用セッション（#349）。

use std::sync::{Arc, Mutex};

use zoo_crypto as core;

use crate::{DeviceAccount, ReceivedRoomKey, Result, ZooCryptoError, bytes32, lock, parse_json};

/// チャンネルごとの Megolm の送信用セッション（[`core::OutboundSession`]）。
///
/// コアの送信用セッションは、まだ保存形式を持たない（送信を行う #361 で足す）。
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
        let message = message.try_into()?;
        let value = lock(&self.inner).encrypt(&account, &message)?;
        Ok(value.to_string())
    }
}

/// Megolm の受信用セッションと、リプレイと版の記録（[`core::InboundSessionStore`]）。
///
/// 受信用セッションは、キーごとに保存する（[`pickle_session`](Self::pickle_session) と
/// [`restore_session`](Self::restore_session)。#359）。リプレイと版の記録は保存しない（#361）。
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

    /// `key` のセッションを、32 バイトの保存鍵で暗号化した保存形式にする（#359）。キーの
    /// セッションがなければ `None`。保存のたびに出力は変わる。
    ///
    /// ロックは端末アカウント、このオブジェクトの順に取る。
    pub fn pickle_session(
        &self,
        account: &DeviceAccount,
        key: SessionRef,
        storage_key: Vec<u8>,
    ) -> Result<Option<String>> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        Ok(lock(&self.inner).pickle_session(&key.into(), &account, &storage_key))
    }

    /// [`pickle_session`](Self::pickle_session) の文字列から、そのキーのセッションを戻す。
    /// 同じキーのセッションがあれば置き換える。戻したキーを返す。保存鍵が違う、別のユーザーや
    /// 端末の値、形式が壊れているときは `InvalidPickle`。
    ///
    /// ロックは端末アカウント、このオブジェクトの順に取る。
    pub fn restore_session(
        &self,
        account: &DeviceAccount,
        pickle: String,
        storage_key: Vec<u8>,
    ) -> Result<SessionRef> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        Ok(lock(&self.inner)
            .restore_session(&pickle, &account, &storage_key)?
            .into())
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
    /// 暗号化した添付（#364）。編集では、元の版の添付をそのまま入れる。
    #[uniffi(default)]
    pub attachments: Vec<AttachmentInfo>,
}

/// Megolm で暗号化する平文に入れる、1 つの添付（[`core::AttachmentInfo`]、#364）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentInfo {
    /// `attachments.id`。
    pub attachment_id: String,
    /// 元のファイル名。
    pub name: String,
    /// 元の MIME。
    pub mimetype: String,
    /// 元の大きさ（バイト）。
    pub size: u64,
    #[uniffi(default)]
    pub width: Option<u32>,
    #[uniffi(default)]
    pub height: Option<u32>,
    /// センシティブ（ぼかして表示する）か。
    #[uniffi(default)]
    pub sensitive: bool,
    /// 鍵、IV、ハッシュ（Matrix の `EncryptedFile` から `url` を除いた JSON）。
    pub file: String,
}

/// [`encrypt_attachment`] の結果（[`core::EncryptedAttachment`]）。
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct EncryptedAttachment {
    /// S3 にアップロードする暗号文。
    pub ciphertext: Vec<u8>,
    /// [`AttachmentInfo::file`] に入れる JSON。鍵を含むので、ログに出さない。
    pub file: String,
}

impl std::fmt::Debug for EncryptedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedAttachment")
            .field("ciphertext_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

/// 添付の中身を、新しい鍵と IV の AES-256-CTR で暗号化する（[`core::encrypt_attachment`]）。
#[uniffi::export]
pub fn encrypt_attachment(plaintext: Vec<u8>) -> EncryptedAttachment {
    let encrypted = core::encrypt_attachment(&plaintext);
    EncryptedAttachment {
        file: serde_json::to_string(&encrypted.file).expect("EncryptedFile は必ず JSON にできる"),
        ciphertext: encrypted.ciphertext,
    }
}

/// S3 から取得した暗号文のハッシュを確かめてから復号する（[`core::decrypt_attachment`]）。
#[uniffi::export]
pub fn decrypt_attachment(file: String, ciphertext: Vec<u8>) -> Result<Vec<u8>> {
    let file = parse_file(&file)?;
    Ok(core::decrypt_attachment(&file, &ciphertext)?)
}

fn parse_file(json: &str) -> Result<core::EncryptedFile> {
    serde_json::from_value(parse_json("file", json)?).map_err(|e| ZooCryptoError::InvalidPayload {
        reason: format!("file の形式が正しくありません: {e}"),
    })
}

impl TryFrom<AttachmentInfo> for core::AttachmentInfo {
    type Error = ZooCryptoError;

    fn try_from(a: AttachmentInfo) -> Result<Self> {
        Ok(Self {
            file: parse_file(&a.file)?,
            attachment_id: a.attachment_id,
            name: a.name,
            mimetype: a.mimetype,
            size: a.size,
            width: a.width,
            height: a.height,
            sensitive: a.sensitive,
        })
    }
}

impl From<core::AttachmentInfo> for AttachmentInfo {
    fn from(a: core::AttachmentInfo) -> Self {
        Self {
            file: serde_json::to_string(&a.file).expect("EncryptedFile は必ず JSON にできる"),
            attachment_id: a.attachment_id,
            name: a.name,
            mimetype: a.mimetype,
            size: a.size,
            width: a.width,
            height: a.height,
            sensitive: a.sensitive,
        }
    }
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
    /// 暗号化した添付（#364）。
    pub attachments: Vec<AttachmentInfo>,
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

impl TryFrom<OutgoingMessage> for core::OutgoingMessage {
    type Error = ZooCryptoError;

    fn try_from(m: OutgoingMessage) -> Result<Self> {
        Ok(Self {
            message_id: m.message_id,
            parent_id: m.parent_id,
            revision: m.revision,
            content: m.content.into(),
            mentions: m.mentions.into(),
            attachments: m
                .attachments
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_>>()?,
        })
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
                attachments: p.attachments.into_iter().map(Into::into).collect(),
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
