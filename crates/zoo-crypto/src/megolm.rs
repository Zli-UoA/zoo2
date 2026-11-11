//! Megolm の送信用セッションと、Megolm で暗号化する平文（`zoo.message`）。
//!
//! 形式は docs/e2ee.md の「Megolm セッションと作り直し」「Megolm で暗号化する平文」
//! 「messages.ciphertext の形式」と同じ。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vodozemac::megolm::{GroupSession, SessionConfig};

use crate::device::{DeviceAccount, MEGOLM_ALGORITHM};
use crate::olm::RoomKey;
use crate::{CryptoError, to_canonical_json};

/// Megolm で暗号化する平文の `type`。
pub const MESSAGE_TYPE: &str = "zoo.message";
/// 1 つの送信用セッションで暗号化するメッセージの数の上限（編集を含む）。
pub const ROTATION_PERIOD_MESSAGES: u64 = 100;
/// 送信用セッションを作ってから作り直すまでの時間（ミリ秒、7 日）。
pub const ROTATION_PERIOD_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// `content.body` の上限（文字数）。平文チャンネルと同じ。
pub const MAX_BODY_CHARS: usize = 40_000;
/// `epoch` と `revision` の上限（2^53-1）。Canonical JSON で表せる整数の最大値と同じ。
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// メッセージの本文（`content`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    /// 本文（MFM）。
    pub body: String,
    /// 本文の形式。`mfm` だけ。
    pub format: String,
    /// `message` か `emote`（`messages.message_kind` と同じ値）。
    pub kind: String,
}

/// メンション。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mentions {
    /// メンションしたユーザー ID。
    pub user_ids: Vec<String>,
    /// `@channel`。
    pub channel: bool,
    /// `@here`。
    pub here: bool,
}

/// Megolm で暗号化する平文（`zoo.message`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePayload {
    /// `zoo.message`。
    #[serde(rename = "type")]
    pub payload_type: String,
    /// メッセージを置くチャンネル。
    pub channel_id: String,
    /// `messages.id`。
    pub message_id: String,
    /// スレッドの親。
    pub parent_id: Option<String>,
    /// 送信用セッションを作ったときの `membership_epoch`。
    pub epoch: u64,
    /// 版の番号。新規送信は 0、編集のたびに 1 増やす。
    pub revision: u64,
    /// 本文。
    pub content: MessageContent,
    /// メンション。
    pub mentions: Mentions,
}

impl MessagePayload {
    /// 形式を確かめる（`type`、`epoch` と `revision` の範囲、`content.format`、`content.kind`、本文の長さ）。
    pub(crate) fn validate(&self) -> Result<(), CryptoError> {
        if self.payload_type != MESSAGE_TYPE {
            return Err(CryptoError::InvalidPayload(format!(
                "type が {MESSAGE_TYPE} ではありません"
            )));
        }
        check_safe_integer("epoch", self.epoch)?;
        check_safe_integer("revision", self.revision)?;
        if self.content.format != "mfm" {
            return Err(CryptoError::InvalidPayload(
                "content.format が mfm ではありません".into(),
            ));
        }
        if !matches!(self.content.kind.as_str(), "message" | "emote") {
            return Err(CryptoError::InvalidPayload(
                "content.kind が message か emote ではありません".into(),
            ));
        }
        if self.content.body.chars().count() > MAX_BODY_CHARS {
            return Err(CryptoError::InvalidPayload(format!(
                "content.body が {MAX_BODY_CHARS} 文字を超えています"
            )));
        }
        Ok(())
    }

    /// 同じ版かを比べるための Canonical JSON。
    ///
    /// [`validate`](Self::validate) を通ったものは必ず変換できるが、範囲外の整数などで
    /// 失敗したときも panic せず [`CryptoError::CanonicalJson`] を返す。
    pub(crate) fn canonical(&self) -> Result<String, CryptoError> {
        let value = serde_json::to_value(self)
            .map_err(|e| CryptoError::InvalidPayload(format!("平文を JSON にできません: {e}")))?;
        to_canonical_json(&value)
    }
}

/// `epoch` や `revision` が [`MAX_SAFE_INTEGER`] 以下かを確かめる。
pub(crate) fn check_safe_integer(field: &str, value: u64) -> Result<(), CryptoError> {
    if value > MAX_SAFE_INTEGER {
        return Err(CryptoError::InvalidPayload(format!(
            "{field} が {MAX_SAFE_INTEGER} を超えています: {value}"
        )));
    }
    Ok(())
}

/// 送るメッセージ。`channel_id` と `epoch` は送信用セッションのものを使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    /// `messages.id`。409 `E2EE_STALE_EPOCH` で送り直すときも同じ ID を使う。
    pub message_id: String,
    /// スレッドの親。
    pub parent_id: Option<String>,
    /// 版の番号。新規送信は 0、編集のたびに 1 増やす。
    pub revision: u64,
    /// 本文。
    pub content: MessageContent,
    /// メンション。
    pub mentions: Mentions,
}

/// チャンネルごとの Megolm の送信用セッション。
pub struct OutboundSession {
    session: GroupSession,
    channel_id: String,
    epoch: u64,
    created_at_ms: u64,
    message_count: u64,
}

impl OutboundSession {
    /// `channel_id` の、epoch が `epoch` の新しい送信用セッションを作る。
    ///
    /// `epoch` は配布先を計算したスナップショットの `membership_epoch`、`now_ms` は今の時刻
    /// （UNIX 時刻のミリ秒）。作ったら [`room_key`](Self::room_key) を配布先の端末に配る。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn new(channel_id: &str, epoch: u64, now_ms: u64) -> Self {
        Self {
            session: GroupSession::new(SessionConfig::version_1()),
            channel_id: channel_id.to_owned(),
            epoch,
            created_at_ms: now_ms,
            message_count: 0,
        }
    }

    /// Megolm セッション ID。
    pub fn session_id(&self) -> String {
        self.session.session_id()
    }

    /// チャンネル。
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// セッションを作ったときの epoch。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// このセッションで暗号化したメッセージの数。
    pub fn message_count(&self) -> u64 {
        self.message_count
    }

    /// 次に暗号化するメッセージのインデックス。
    pub fn message_index(&self) -> u32 {
        self.session.message_index()
    }

    /// 配布するセッション鍵。**今のインデックスから**のもので、これより前のメッセージは読めない。
    pub fn room_key(&self) -> RoomKey {
        RoomKey {
            channel_id: self.channel_id.clone(),
            epoch: self.epoch,
            session_id: self.session_id(),
            session_key: self.session.session_key().to_base64(),
        }
    }

    /// 次の送信の前に作り直すべきか。
    ///
    /// 暗号化した数が [`ROTATION_PERIOD_MESSAGES`] に達した、作ってから
    /// [`ROTATION_PERIOD_MS`] たった、チャンネルの `membership_epoch`（`current_epoch`）が
    /// 作ったときの値と違う、のどれかで `true`。配布先から端末やユーザーを外したときは、
    /// これによらず作り直す。時計が戻ったときは、たった時間を 0 とみなす。
    pub fn needs_rotation(&self, now_ms: u64, current_epoch: u64) -> bool {
        self.message_count >= ROTATION_PERIOD_MESSAGES
            || now_ms.saturating_sub(self.created_at_ms) >= ROTATION_PERIOD_MS
            || current_epoch != self.epoch
    }

    /// `message` を暗号化し、`messages.ciphertext` に入れる JSON を返す。
    ///
    /// ```json
    /// {"algorithm": "m.megolm.v1.aes-sha2", "sender_key", "device_id", "session_id", "ciphertext"}
    /// ```
    ///
    /// 平文の `channel_id` と `epoch` にはこのセッションの値を入れる。行の `epoch` にも
    /// [`epoch`](Self::epoch) を入れて送る。
    pub fn encrypt(
        &mut self,
        account: &DeviceAccount,
        message: &OutgoingMessage,
    ) -> Result<Value, CryptoError> {
        let payload = MessagePayload {
            payload_type: MESSAGE_TYPE.to_owned(),
            channel_id: self.channel_id.clone(),
            message_id: message.message_id.clone(),
            parent_id: message.parent_id.clone(),
            epoch: self.epoch,
            revision: message.revision,
            content: message.content.clone(),
            mentions: message.mentions.clone(),
        };
        payload.validate()?;
        let plaintext = serde_json::to_vec(&payload)
            .expect("文字列、整数、真偽値だけの構造体は必ず JSON にできる");
        let ciphertext = self.session.encrypt(plaintext);
        self.message_count += 1;
        Ok(json!({
            "algorithm": MEGOLM_ALGORITHM,
            "sender_key": account.identity_keys().curve25519.to_base64(),
            "device_id": account.device_id(),
            "session_id": self.session_id(),
            "ciphertext": ciphertext.to_base64(),
        }))
    }
}

impl std::fmt::Debug for OutboundSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundSession")
            .field("session_id", &self.session_id())
            .field("channel_id", &self.channel_id)
            .field("epoch", &self.epoch)
            .field("created_at_ms", &self.created_at_ms)
            .field("message_count", &self.message_count)
            .finish_non_exhaustive()
    }
}
