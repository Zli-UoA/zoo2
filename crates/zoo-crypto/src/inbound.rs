//! Megolm の受信用セッションの保存と、メッセージを復号したときの検証。
//!
//! 規則は docs/e2ee.md の「受信時の検証」と同じ。
//!
//! - 受信用セッションは `(channel_id, sender_key, session_id)` をキーにし、入手経路
//!   （Olm 由来、バックアップ由来）を 1 つずつまで持つ。
//! - 復号したら、チャンネル、行、種別、送信者と端末、epoch、リプレイ、版を照合する。

use std::collections::HashMap;

use serde_json::Value;
use vodozemac::Curve25519PublicKey;
use vodozemac::megolm::{
    DecryptionError, ExportedSessionKey, InboundGroupSession, MegolmMessage, SessionConfig,
    SessionOrdering,
};

use crate::device::{DeviceAccount, MEGOLM_ALGORITHM};
use crate::megolm::{MessagePayload, OutboundSession};
use crate::olm::ReceivedRoomKey;
use crate::{CryptoError, Rejection};

/// 受信用セッションをどこから手に入れたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOrigin {
    /// Olm（`zoo.room_key`）で送信端末から受け取った。自分の送信用セッションもこれ。
    Olm,
    /// 鍵バックアップから取り込んだ。送信者を確かめられない。
    Backup,
}

/// 受信用セッションを保存しようとした結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    /// 同じキーと入手経路のセッションがなかったので保存した。
    Added,
    /// 保存済みのセッションを新しい方に置き換えた。
    Replaced,
    /// 保存済みのセッションを残し、新しい方を捨てた。
    Kept,
}

/// 鍵バックアップから取り出した 1 つのセッション（#351 が復号して渡す）。
///
/// `sender_key`、`sender_user_id`、`sender_device_id` は平文で、誰でも好きな値を入れられる。
/// 保存のキーと照合にだけ使い、送信者が誰かの判定には使わない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSession {
    /// セッションを使うチャンネル。
    pub channel_id: String,
    /// 元の送信端末の識別鍵（と書かれている値）。
    pub sender_key: Curve25519PublicKey,
    /// 宣言された Megolm セッション ID。
    pub session_id: String,
    /// 送信ユーザー（と書かれている値）。
    pub sender_user_id: String,
    /// 送信端末（と書かれている値）。
    pub sender_device_id: String,
    /// セッションの epoch。
    pub epoch: u64,
    /// vodozemac の `ExportedSessionKey` の base64。
    pub session_key: String,
}

/// 復号する `messages` の行。値はサーバーの行のものをそのまま入れる。
#[derive(Debug, Clone, Copy)]
pub struct EncryptedMessage<'a> {
    /// 行の `channel_id`。
    pub channel_id: &'a str,
    /// 行の `id`。
    pub message_id: &'a str,
    /// 行の `parent_id`。
    pub parent_id: Option<&'a str>,
    /// 行の `user_id`（送信者）。
    pub user_id: &'a str,
    /// 行の `message_kind`。
    pub message_kind: &'a str,
    /// 行の `epoch`。
    pub epoch: u64,
    /// 行の `revision`。
    pub revision: u64,
    /// 行の `ciphertext`。
    pub ciphertext: &'a Value,
}

/// 版の照合の結果。どれでも破棄はしない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionStatus {
    /// 見た中で最新の版。表示を更新してよい。
    Latest,
    /// これより新しい版（`latest`）を見ている。表示は最新の版のままにし、
    /// 「サーバーが古い版を返した」と警告する。
    Stale {
        /// 見た中で最大の `revision`。
        latest: u64,
    },
    /// 同じ `revision` で中身が違う。先に見た方を表示したまま警告する。
    Conflict,
}

/// 検証を通ったメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedMessage {
    /// 復号した平文。
    pub payload: MessagePayload,
    /// Megolm のメッセージインデックス。
    pub message_index: u32,
    /// 復号に使ったセッションの入手経路。`Backup` なら「送信者を確かめられない」の印を付ける。
    pub origin: SessionOrigin,
    /// 送信端末が検証済みか。Olm 由来のセッションだけで判定し、`Backup` なら常に `false`。
    /// `false` なら「未検証の送信者」と表示する（破棄はしない）。
    pub sender_verified: bool,
    /// 版の照合の結果。
    pub revision: RevisionStatus,
}

struct StoredSession {
    session: InboundGroupSession,
    sender_user_id: String,
    sender_device_id: String,
    epoch: u64,
    sender_verified: bool,
}

#[derive(Default)]
struct Slot {
    olm: Option<StoredSession>,
    backup: Option<StoredSession>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct SessionKeyId {
    channel_id: String,
    sender_key: String,
    session_id: String,
}

/// Megolm の受信用セッションと、リプレイと版の記録。
///
/// I/O を持たないメモリー上の状態。保存（暗号ストア）は #359 で足す。
#[derive(Default)]
pub struct InboundSessionStore {
    sessions: HashMap<SessionKeyId, Slot>,
    /// `(session_id, message_index)` ごとに、最初に復号した `message_id`。
    first_message_ids: HashMap<(String, u32), String>,
    /// `message_id` ごとに、見た中で最大の `revision` と、その平文の Canonical JSON。
    revisions: HashMap<String, (u64, String)>,
}

impl std::fmt::Debug for InboundSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundSessionStore")
            .field("sessions", &self.sessions.len())
            .field("first_message_ids", &self.first_message_ids.len())
            .field("revisions", &self.revisions.len())
            .finish()
    }
}

impl InboundSessionStore {
    /// 空の状態を作る。
    pub fn new() -> Self {
        Self::default()
    }

    /// Olm で受け取って検証したセッション鍵を、Olm 由来として保存する（受信時の検証の手順 5）。
    ///
    /// 同じキーの Olm 由来のセッションがあれば、送信ユーザーか送信端末が違うときは新しい方を
    /// 捨て、同じなら first known index が小さい方を残す。バックアップ由来のセッションとは
    /// 比べない。
    pub fn add_room_key(&mut self, room_key: ReceivedRoomKey) -> AddOutcome {
        let key = SessionKeyId {
            channel_id: room_key.channel_id,
            sender_key: room_key.sender_key.to_base64(),
            session_id: room_key.session.session_id(),
        };
        let new = StoredSession {
            session: room_key.session,
            sender_user_id: room_key.sender_user_id,
            sender_device_id: room_key.sender_device_id,
            epoch: room_key.epoch,
            sender_verified: room_key.sender_verified,
        };
        let slot = &mut self.sessions.entry(key).or_default().olm;
        match slot {
            None => {
                *slot = Some(new);
                AddOutcome::Added
            }
            Some(old) => {
                if old.sender_user_id != new.sender_user_id
                    || old.sender_device_id != new.sender_device_id
                {
                    AddOutcome::Kept
                } else if new.session.first_known_index() < old.session.first_known_index() {
                    *old = new;
                    AddOutcome::Replaced
                } else {
                    AddOutcome::Kept
                }
            }
        }
    }

    /// 自分の送信用セッションを、自分の端末で読めるように Olm 由来として保存する。
    ///
    /// 送信用セッションを作った直後（インデックスが 0 のうち）に呼ぶ。
    pub fn add_outbound(
        &mut self,
        account: &DeviceAccount,
        outbound: &OutboundSession,
    ) -> AddOutcome {
        let room_key = outbound.room_key();
        let session_key = vodozemac::megolm::SessionKey::from_base64(&room_key.session_key)
            .expect("送信用セッションの鍵は必ず読める");
        self.add_room_key(ReceivedRoomKey {
            channel_id: room_key.channel_id,
            epoch: room_key.epoch,
            sender_key: account.identity_keys().curve25519,
            sender_user_id: account.user_id().to_owned(),
            sender_device_id: account.device_id().to_owned(),
            sender_verified: true,
            session: InboundGroupSession::new(&session_key, SessionConfig::version_1()),
        })
    }

    /// 鍵バックアップから取り出したセッションを、バックアップ由来として保存する。
    ///
    /// - `session_key` から導いた `session_id` が宣言と違えば捨てる（エラーを返す）。
    /// - Olm 由来のセッションは置き換えず、比べもしない。
    /// - 同じキーのバックアップ由来のセッションがあれば、送信ユーザーか送信端末が違うときは
    ///   新しい方を捨て、同じなら vodozemac の `compare` が `Better` のときだけ置き換える。
    pub fn import_backup_session(
        &mut self,
        backup: BackupSession,
    ) -> Result<AddOutcome, CryptoError> {
        let exported = ExportedSessionKey::from_base64(&backup.session_key)
            .map_err(|e| CryptoError::InvalidPayload(format!("session_key が壊れています: {e}")))?;
        let session = InboundGroupSession::import(&exported, SessionConfig::version_1());
        if session.session_id() != backup.session_id {
            return Err(CryptoError::InvalidPayload(
                "session_key から導いた session_id が宣言と違います".into(),
            ));
        }
        let key = SessionKeyId {
            channel_id: backup.channel_id,
            sender_key: backup.sender_key.to_base64(),
            session_id: backup.session_id,
        };
        let mut new = StoredSession {
            session,
            sender_user_id: backup.sender_user_id,
            sender_device_id: backup.sender_device_id,
            epoch: backup.epoch,
            sender_verified: false,
        };
        let slot = &mut self.sessions.entry(key).or_default().backup;
        Ok(match slot {
            None => {
                *slot = Some(new);
                AddOutcome::Added
            }
            Some(old) => {
                if old.sender_user_id != new.sender_user_id
                    || old.sender_device_id != new.sender_device_id
                {
                    AddOutcome::Kept
                } else if new.session.compare(&mut old.session) == SessionOrdering::Better {
                    *old = new;
                    AddOutcome::Replaced
                } else {
                    AddOutcome::Kept
                }
            }
        })
    }

    /// `messages` の行を復号し、docs/e2ee.md の「受信時の検証」の表のとおり照合する。
    ///
    /// 照合に失敗したら [`CryptoError::Rejected`] を返し、呼び出し側はメッセージを破棄する。
    /// 鍵がまだ届いていなければ [`CryptoError::MissingRoomKey`] を返す。
    /// 同じメッセージの再取得と再復号は通る。
    pub fn decrypt(&mut self, row: &EncryptedMessage<'_>) -> Result<DecryptedMessage, CryptoError> {
        let ciphertext = ParsedCiphertext::parse(row.ciphertext)?;
        let key = SessionKeyId {
            channel_id: row.channel_id.to_owned(),
            sender_key: ciphertext.sender_key.to_base64(),
            session_id: ciphertext.session_id.clone(),
        };
        let missing = |message_index| CryptoError::MissingRoomKey {
            session_id: ciphertext.session_id.clone(),
            message_index,
        };
        let slot = self.sessions.get_mut(&key).ok_or_else(|| missing(None))?;

        // Olm 由来を先に使う。バックアップ由来は、Olm 由来がないか、
        // その first known index より前のメッセージのときだけ使う。
        let mut decrypted = None;
        if let Some(stored) = slot.olm.as_mut() {
            match stored.session.decrypt(&ciphertext.message) {
                Ok(message) => decrypted = Some((message, SessionOrigin::Olm)),
                Err(DecryptionError::UnknownMessageIndex(..)) => {}
                Err(e) => return Err(CryptoError::Megolm(e.to_string())),
            }
        }
        if decrypted.is_none()
            && let Some(stored) = slot.backup.as_mut()
        {
            match stored.session.decrypt(&ciphertext.message) {
                Ok(message) => decrypted = Some((message, SessionOrigin::Backup)),
                Err(DecryptionError::UnknownMessageIndex(_, index)) => {
                    return Err(missing(Some(index)));
                }
                Err(e) => return Err(CryptoError::Megolm(e.to_string())),
            }
        }
        let Some((message, origin)) = decrypted else {
            return Err(missing(Some(ciphertext.message.message_index())));
        };
        let stored = match origin {
            SessionOrigin::Olm => slot.olm.as_ref(),
            SessionOrigin::Backup => slot.backup.as_ref(),
        }
        .expect("復号に使ったセッションはある");

        let payload: MessagePayload = serde_json::from_slice(&message.plaintext)
            .map_err(|e| CryptoError::InvalidPayload(format!("平文の形式が違います: {e}")))?;
        payload.validate()?;

        let reject = |reason| Err(CryptoError::Rejected(reason));
        // チャンネル：セッションは行の channel_id で探したので、ペイロードとだけ比べればよい。
        if payload.channel_id != row.channel_id {
            return reject(Rejection::ChannelMismatch);
        }
        if payload.message_id != row.message_id {
            return reject(Rejection::MessageIdMismatch);
        }
        if payload.parent_id.as_deref() != row.parent_id {
            return reject(Rejection::ParentMismatch);
        }
        if payload.content.kind != row.message_kind {
            return reject(Rejection::KindMismatch);
        }
        if stored.sender_user_id != row.user_id {
            return reject(Rejection::SenderMismatch);
        }
        if stored.sender_device_id != ciphertext.device_id {
            return reject(Rejection::DeviceMismatch);
        }
        if payload.epoch != row.epoch || stored.epoch != row.epoch {
            return reject(Rejection::EpochMismatch);
        }
        if payload.revision != row.revision {
            return reject(Rejection::RevisionMismatch);
        }
        let sender_verified = origin == SessionOrigin::Olm && stored.sender_verified;

        // リプレイ：同じインデックスに別の message_id が来たら破棄。同じなら再取得として通す。
        let index_key = (ciphertext.session_id, message.message_index);
        if let Some(first) = self.first_message_ids.get(&index_key) {
            if first != row.message_id {
                return reject(Rejection::Replay {
                    first_message_id: first.clone(),
                });
            }
        } else {
            self.first_message_ids
                .insert(index_key, row.message_id.to_owned());
        }

        let canonical = payload.canonical();
        let revision = match self.revisions.get_mut(row.message_id) {
            Some((latest, _)) if *latest > payload.revision => {
                RevisionStatus::Stale { latest: *latest }
            }
            Some((latest, seen)) if *latest == payload.revision => {
                if *seen == canonical {
                    RevisionStatus::Latest
                } else {
                    RevisionStatus::Conflict
                }
            }
            _ => {
                self.revisions
                    .insert(row.message_id.to_owned(), (payload.revision, canonical));
                RevisionStatus::Latest
            }
        };

        Ok(DecryptedMessage {
            payload,
            message_index: message.message_index,
            origin,
            sender_verified,
            revision,
        })
    }
}

struct ParsedCiphertext {
    sender_key: Curve25519PublicKey,
    device_id: String,
    session_id: String,
    message: MegolmMessage,
}

impl ParsedCiphertext {
    fn parse(value: &Value) -> Result<Self, CryptoError> {
        let field = |name: &str| {
            value.get(name).and_then(Value::as_str).ok_or_else(|| {
                CryptoError::InvalidPayload(format!("ciphertext.{name} がありません"))
            })
        };
        if field("algorithm")? != MEGOLM_ALGORITHM {
            return Err(CryptoError::InvalidPayload(format!(
                "ciphertext.algorithm が {MEGOLM_ALGORITHM} ではありません"
            )));
        }
        Ok(Self {
            sender_key: Curve25519PublicKey::from_base64(field("sender_key")?)?,
            device_id: field("device_id")?.to_owned(),
            session_id: field("session_id")?.to_owned(),
            message: MegolmMessage::from_base64(field("ciphertext")?).map_err(|e| {
                CryptoError::InvalidPayload(format!("Megolm メッセージが壊れています: {e}"))
            })?,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vodozemac::megolm::{GroupSession, GroupSessionPickle};

    use super::*;
    use crate::{Mentions, MessageContent};

    const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
    const ALICE_DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const MALLORY: &str = "00000000-0000-0000-0000-00000000000c";
    const CHANNEL: &str = "c";

    fn payload(message_id: &str) -> MessagePayload {
        MessagePayload {
            payload_type: crate::MESSAGE_TYPE.into(),
            channel_id: CHANNEL.into(),
            message_id: message_id.into(),
            parent_id: None,
            epoch: 1,
            revision: 0,
            content: MessageContent {
                body: message_id.into(),
                format: "mfm".into(),
                kind: "message".into(),
            },
            mentions: Mentions::default(),
        }
    }

    fn ciphertext(
        alice: &DeviceAccount,
        session: &mut GroupSession,
        payload: &MessagePayload,
    ) -> Value {
        json!({
            "algorithm": MEGOLM_ALGORITHM,
            "sender_key": alice.identity_keys().curve25519.to_base64(),
            "device_id": alice.device_id(),
            "session_id": session.session_id(),
            "ciphertext": session.encrypt(serde_json::to_vec(payload).unwrap()).to_base64(),
        })
    }

    fn room_key(
        alice: &DeviceAccount,
        session: &GroupSession,
        sender_user_id: &str,
    ) -> ReceivedRoomKey {
        ReceivedRoomKey {
            channel_id: CHANNEL.into(),
            epoch: 1,
            sender_key: alice.identity_keys().curve25519,
            sender_user_id: sender_user_id.into(),
            sender_device_id: alice.device_id().into(),
            sender_verified: false,
            session: InboundGroupSession::new(&session.session_key(), SessionConfig::version_1()),
        }
    }

    fn row<'a>(message_id: &'a str, ciphertext: &'a Value) -> EncryptedMessage<'a> {
        EncryptedMessage {
            channel_id: CHANNEL,
            message_id,
            parent_id: None,
            user_id: ALICE,
            message_kind: "message",
            epoch: 1,
            revision: 0,
            ciphertext,
        }
    }

    #[test]
    fn rejects_another_message_at_the_same_index() {
        let alice = DeviceAccount::new(ALICE, ALICE_DEVICE).unwrap();
        let mut session = GroupSession::new(SessionConfig::version_1());
        // 同じ状態から 2 つ目の送信用セッションを作り、同じインデックスで別のメッセージを暗号化する。
        let pickle: GroupSessionPickle = session.pickle();
        let mut fork = GroupSession::from_pickle(pickle);
        let mut store = InboundSessionStore::new();
        store.add_room_key(room_key(&alice, &session, ALICE));

        let first = ciphertext(&alice, &mut session, &payload("m1"));
        let replayed = ciphertext(&alice, &mut fork, &payload("m2"));
        assert_eq!(store.decrypt(&row("m1", &first)).unwrap().message_index, 0);
        assert!(matches!(
            store.decrypt(&row("m2", &replayed)),
            Err(CryptoError::Rejected(Rejection::Replay { first_message_id })) if first_message_id == "m1"
        ));
        // 最初のメッセージの再取得と再復号は通る。
        store.decrypt(&row("m1", &first)).unwrap();
        store.decrypt(&row("m1", &first)).unwrap();
    }

    #[test]
    fn keeps_the_first_sender_for_the_same_key() {
        let alice = DeviceAccount::new(ALICE, ALICE_DEVICE).unwrap();
        let mut session = GroupSession::new(SessionConfig::version_1());
        let mut store = InboundSessionStore::new();
        assert_eq!(
            store.add_room_key(room_key(&alice, &session, ALICE)),
            AddOutcome::Added
        );
        // 同じキーで送信ユーザーが違うものは、first known index によらず捨てる。
        assert_eq!(
            store.add_room_key(room_key(&alice, &session, MALLORY)),
            AddOutcome::Kept
        );
        let first = ciphertext(&alice, &mut session, &payload("m1"));
        store.decrypt(&row("m1", &first)).unwrap();
    }
}
