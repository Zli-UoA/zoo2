//! Olm（to-device メッセージ）のセッションと、Megolm セッション鍵の配布（`zoo.room_key`）。
//!
//! 形式は docs/e2ee.md の「Olm（to-device メッセージ）」、受け取ったときの検証は
//! 「受信時の検証」の `zoo.room_key` の手順 1〜4 と同じ。

use serde_json::{Value, json};
use vodozemac::megolm::{InboundGroupSession, SessionConfig as MegolmSessionConfig, SessionKey};
use vodozemac::olm::{OlmMessage, Session, SessionConfig};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, base64_decode, base64_encode};

use crate::device::{
    DeviceAccount, DeviceKeys, MEGOLM_ALGORITHM, OLM_ALGORITHM, verify_device_keys,
};
use crate::megolm::check_safe_integer;
use crate::{CryptoError, Rejection, verify_json_signature};

/// Megolm セッション鍵の配布に使う、Olm で暗号化する平文の `type`。
pub const ROOM_KEY_TYPE: &str = "zoo.room_key";

/// 端末の組ごとの Olm セッション。
///
/// vodozemac の [`Session`] に、相手の識別鍵を組み合わせたもの。
/// 状態は暗号化と復号のたびに進むので、呼び出し側は使ったあとに保存し直す。
pub struct OlmSession {
    session: Session,
    their_identity_key: Curve25519PublicKey,
}

impl OlmSession {
    /// セッション ID。
    pub fn session_id(&self) -> String {
        self.session.session_id()
    }

    /// 相手の端末の識別鍵（Curve25519）。
    pub fn their_identity_key(&self) -> Curve25519PublicKey {
        self.their_identity_key
    }
}

impl std::fmt::Debug for OlmSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OlmSession")
            .field("session_id", &self.session_id())
            .field("their_identity_key", &self.their_identity_key)
            .finish_non_exhaustive()
    }
}

/// Megolm セッション鍵の配布（`zoo.room_key` の `content`）。
///
/// `Debug` は `session_key` を出さない。
#[derive(Clone, PartialEq, Eq)]
pub struct RoomKey {
    /// セッションを使うチャンネル。
    pub channel_id: String,
    /// 送信用セッションを作ったときの `membership_epoch`。
    pub epoch: u64,
    /// Megolm セッション ID。
    pub session_id: String,
    /// Megolm のセッション鍵（vodozemac の `SessionKey` の base64）。
    pub session_key: String,
}

impl std::fmt::Debug for RoomKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomKey")
            .field("channel_id", &self.channel_id)
            .field("epoch", &self.epoch)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl RoomKey {
    fn to_json(&self) -> Value {
        json!({
            "algorithm": MEGOLM_ALGORITHM,
            "channel_id": self.channel_id,
            "epoch": self.epoch,
            "session_id": self.session_id,
            "session_key": self.session_key,
        })
    }
}

impl DeviceAccount {
    /// `keys/claim` で払い出された相手の端末のワンタイム鍵（またはフォールバック鍵）で、
    /// Olm セッションを作る。
    ///
    /// `recipient` は [`verify_device_keys`] で検証した相手の端末鍵。`one_time_key` は
    /// 応答の `signed_curve25519:<key_id>` の値（`{"key", "signatures", "fallback"?}`）で、
    /// その署名を `recipient` の端末の署名鍵で検証してから使う。
    pub fn create_outbound_olm_session(
        &self,
        recipient: &DeviceKeys,
        one_time_key: &Value,
    ) -> Result<OlmSession, CryptoError> {
        verify_json_signature(
            one_time_key,
            &recipient.user_id,
            &format!("ed25519:{}", recipient.device_id),
            &recipient.ed25519,
        )?;
        let key = one_time_key
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| CryptoError::InvalidPayload("ワンタイム鍵に key がありません".into()))?;
        let one_time_key = Curve25519PublicKey::from_base64(key)?;
        let session = self
            .account()
            .create_outbound_session(
                SessionConfig::version_1(),
                recipient.curve25519,
                one_time_key,
            )
            .map_err(|e| CryptoError::Olm(e.to_string()))?;
        Ok(OlmSession {
            session,
            their_identity_key: recipient.curve25519,
        })
    }

    /// `room_key` を `recipient` の端末に送る to-device メッセージの `content` を作る。
    ///
    /// ```json
    /// {"algorithm": "m.olm.v1.curve25519-aes-sha2", "sender_key", "ciphertext": {"<受信端末の識別鍵>": {"type", "body"}}}
    /// ```
    ///
    /// `session` は `recipient` の識別鍵とのセッションでなければならない。
    pub fn encrypt_room_key(
        &self,
        session: &mut OlmSession,
        recipient: &DeviceKeys,
        room_key: &RoomKey,
    ) -> Result<Value, CryptoError> {
        if session.their_identity_key != recipient.curve25519 {
            return Err(CryptoError::Olm(
                "Olm セッションの相手が宛先の端末ではありません".into(),
            ));
        }
        let own = self.identity_keys();
        let plaintext = json!({
            "type": ROOM_KEY_TYPE,
            "content": room_key.to_json(),
            "sender": self.user_id(),
            "sender_device": self.device_id(),
            "keys": { "ed25519": own.ed25519.to_base64() },
            "recipient": recipient.user_id,
            "recipient_keys": { "ed25519": recipient.ed25519.to_base64() },
            "sender_device_keys": self.device_keys(),
        });
        let message = session
            .session
            .encrypt(plaintext.to_string())
            .map_err(|e| CryptoError::Olm(e.to_string()))?;
        let (message_type, body) = message.to_parts();
        Ok(json!({
            "algorithm": OLM_ALGORITHM,
            "sender_key": own.curve25519.to_base64(),
            "ciphertext": {
                recipient.curve25519.to_base64(): { "type": message_type, "body": base64_encode(body) },
            },
        }))
    }

    /// 受け取った to-device メッセージの `content` を復号する。
    ///
    /// `sessions` は保存してある Olm セッション。送信端末の識別鍵（`sender_key`）と
    /// 一致するセッションで復号し、なければ（pre-key メッセージなら）新しいセッションを作って
    /// `sessions` に足す。新しいセッションを作るとアカウントのワンタイム鍵を 1 つ使うので、
    /// 呼び出し側は、成功したらアカウントと `sessions` の両方を保存し直す。
    ///
    /// 返した [`OlmDecrypted`] はまだ検証していない。[`OlmDecrypted::verify_room_key`] で
    /// 送信端末を確かめてから使う。
    pub fn decrypt_to_device(
        &mut self,
        sessions: &mut Vec<OlmSession>,
        content: &Value,
    ) -> Result<OlmDecrypted, CryptoError> {
        if content.get("algorithm").and_then(Value::as_str) != Some(OLM_ALGORITHM) {
            return Err(CryptoError::InvalidPayload(format!(
                "algorithm が {OLM_ALGORITHM} ではありません"
            )));
        }
        let sender_key = content
            .get("sender_key")
            .and_then(Value::as_str)
            .ok_or_else(|| CryptoError::InvalidPayload("sender_key がありません".into()))?;
        let sender_key = Curve25519PublicKey::from_base64(sender_key)?;
        let own_key = self.identity_keys().curve25519.to_base64();
        let ciphertext = content
            .get("ciphertext")
            .and_then(|c| c.get(&own_key))
            .ok_or_else(|| {
                CryptoError::InvalidPayload("この端末あての ciphertext がありません".into())
            })?;
        let message_type = ciphertext
            .get("type")
            .and_then(Value::as_u64)
            .and_then(|t| usize::try_from(t).ok())
            .ok_or_else(|| CryptoError::InvalidPayload("ciphertext.type がありません".into()))?;
        let body = ciphertext
            .get("body")
            .and_then(Value::as_str)
            .and_then(|b| base64_decode(b).ok())
            .ok_or_else(|| {
                CryptoError::InvalidPayload("ciphertext.body が base64 ではありません".into())
            })?;
        let message = OlmMessage::from_parts(message_type, &body).map_err(|e| {
            CryptoError::InvalidPayload(format!("Olm メッセージが壊れています: {e}"))
        })?;

        let plaintext = self.decrypt_olm_message(sessions, sender_key, &message)?;
        let plaintext: Value = serde_json::from_slice(&plaintext)
            .map_err(|e| CryptoError::InvalidPayload(format!("平文が JSON ではありません: {e}")))?;
        if !plaintext.is_object() {
            return Err(CryptoError::InvalidPayload(
                "平文が JSON オブジェクトではありません".into(),
            ));
        }
        Ok(OlmDecrypted {
            sender_key,
            plaintext,
        })
    }

    fn decrypt_olm_message(
        &mut self,
        sessions: &mut Vec<OlmSession>,
        sender_key: Curve25519PublicKey,
        message: &OlmMessage,
    ) -> Result<Vec<u8>, CryptoError> {
        let candidates = sessions
            .iter_mut()
            .filter(|s| s.their_identity_key == sender_key);
        match message {
            OlmMessage::PreKey(pre_key) => {
                // pre-key メッセージは、同じ ID のセッションがあればそれだけで復号する。
                // 復号できなくても新しいセッションは作らない（同じワンタイム鍵の再利用になる）。
                let session_id = pre_key.session_id();
                let mut candidates = candidates;
                if let Some(session) = candidates.find(|s| s.session_id() == session_id) {
                    return session
                        .session
                        .decrypt(message)
                        .map_err(|e| CryptoError::Olm(e.to_string()));
                }
                let result = self
                    .account_mut()
                    .create_inbound_session(SessionConfig::version_1(), sender_key, pre_key)
                    .map_err(|e| CryptoError::Olm(e.to_string()))?;
                sessions.push(OlmSession {
                    session: result.session,
                    their_identity_key: sender_key,
                });
                Ok(result.plaintext)
            }
            OlmMessage::Normal(_) => {
                for session in candidates {
                    if let Ok(plaintext) = session.session.decrypt(message) {
                        return Ok(plaintext);
                    }
                }
                Err(CryptoError::Olm(
                    "送信端末とのどの Olm セッションでも復号できません".into(),
                ))
            }
        }
    }
}

/// [`DeviceAccount::decrypt_to_device`] で復号した、まだ検証していない平文。
///
/// 平文はセッション鍵を含むので、`Debug` は `sender_key`、`type`、`sender`、`sender_device` だけを出す。
#[derive(Clone)]
pub struct OlmDecrypted {
    sender_key: Curve25519PublicKey,
    plaintext: Value,
}

impl std::fmt::Debug for OlmDecrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OlmDecrypted")
            .field("sender_key", &self.sender_key)
            .field("type", &self.plaintext.get("type").and_then(Value::as_str))
            .field("sender", &self.sender())
            .field("sender_device", &self.sender_device())
            .finish_non_exhaustive()
    }
}

/// 送信端末が、Shape（または `keys/query` の応答）にあるか。
#[derive(Debug, Clone, Copy)]
pub enum ListedDevice<'a> {
    /// Shape の行か `keys/query` の応答にあった、その端末の `device_keys`。
    Listed(&'a Value),
    /// Shape にも `keys/query` の応答にもない（削除済みか存在しない）。
    NotListed,
}

/// 送信端末を検証済みにするための、送信ユーザーのクロス署名の情報（手順 4）。
///
/// マスター鍵を信頼し、自己署名鍵がそのマスター鍵で署名されていることは、呼び出し側が
/// 確かめておく（#350）。ここでは、端末鍵がその自己署名鍵で署名されていることだけを確かめる。
#[derive(Debug, Clone)]
pub struct SenderTrust {
    /// 送信ユーザーの、信頼したマスター鍵で署名された自己署名鍵。
    pub self_signing_key: Ed25519PublicKey,
    /// `e2ee_key_signatures` にある、自己署名鍵による端末鍵への署名（base64）。
    /// 端末鍵の JSON に署名がないときだけ使う。
    pub signature: Option<String>,
}

/// 検証を通った Megolm セッション鍵。[`InboundSessionStore::add_room_key`](crate::InboundSessionStore::add_room_key) で保存する。
pub struct ReceivedRoomKey {
    pub(crate) channel_id: String,
    pub(crate) epoch: u64,
    pub(crate) sender_key: Curve25519PublicKey,
    pub(crate) sender_user_id: String,
    pub(crate) sender_device_id: String,
    pub(crate) sender_signing_key: Ed25519PublicKey,
    pub(crate) sender_verified: bool,
    pub(crate) session: InboundGroupSession,
}

impl ReceivedRoomKey {
    /// セッションを使うチャンネル。
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// セッションの epoch。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Megolm セッション ID。
    pub fn session_id(&self) -> String {
        self.session.session_id()
    }

    /// Olm セッションの相手（送信端末）の識別鍵。
    pub fn sender_key(&self) -> Curve25519PublicKey {
        self.sender_key
    }

    /// 送信ユーザー。
    pub fn sender_user_id(&self) -> &str {
        &self.sender_user_id
    }

    /// 送信端末。
    pub fn sender_device_id(&self) -> &str {
        &self.sender_device_id
    }

    /// 送信端末をクロス署名で検証できたか。`false` なら「未検証の送信者」として扱う。
    pub fn sender_verified(&self) -> bool {
        self.sender_verified
    }
}

impl std::fmt::Debug for ReceivedRoomKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedRoomKey")
            .field("channel_id", &self.channel_id)
            .field("epoch", &self.epoch)
            .field("session_id", &self.session_id())
            .field("sender_key", &self.sender_key)
            .field("sender_user_id", &self.sender_user_id)
            .field("sender_device_id", &self.sender_device_id)
            .field("sender_verified", &self.sender_verified)
            .finish_non_exhaustive()
    }
}

impl OlmDecrypted {
    /// Olm セッションの相手の識別鍵。
    pub fn sender_key(&self) -> Curve25519PublicKey {
        self.sender_key
    }

    /// 平文の `sender`。呼び出し側が Shape や `keys/query` で送信端末を探すのに使う。
    pub fn sender(&self) -> Option<&str> {
        self.plaintext.get("sender").and_then(Value::as_str)
    }

    /// 平文の `sender_device`。
    pub fn sender_device(&self) -> Option<&str> {
        self.plaintext.get("sender_device").and_then(Value::as_str)
    }

    /// 復号した平文。
    pub fn plaintext(&self) -> &Value {
        &self.plaintext
    }

    /// `zoo.room_key` を、docs/e2ee.md の「受信時の検証」の手順 1〜4 で確かめる。
    ///
    /// - `account` は受け取った自分の端末。
    /// - `listed` は、平文の `sender` と `sender_device` で探した Shape の行か `keys/query` の応答。
    /// - `trust` は手順 4 の判定に使う送信ユーザーの自己署名鍵。クロス署名を確かめられないときは `None`。
    ///
    /// どれかに失敗すれば [`CryptoError::Rejected`] などのエラーを返し、呼び出し側は破棄する。
    /// 送信端末を検証済みにできなかっただけなら、エラーにせず
    /// [`ReceivedRoomKey::sender_verified`] を `false` にする。
    pub fn verify_room_key(
        &self,
        account: &DeviceAccount,
        listed: ListedDevice<'_>,
        trust: Option<&SenderTrust>,
    ) -> Result<ReceivedRoomKey, CryptoError> {
        let p = &self.plaintext;
        if p.get("type").and_then(Value::as_str) != Some(ROOM_KEY_TYPE) {
            return Err(CryptoError::InvalidPayload(format!(
                "type が {ROOM_KEY_TYPE} ではありません"
            )));
        }
        let str_field = |name: &str| {
            p.get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| CryptoError::InvalidPayload(format!("{name} がありません")))
        };
        let sender = str_field("sender")?;
        let sender_device = str_field("sender_device")?;

        // 手順 1：送信端末の端末鍵を決め、Olm セッションの相手の識別鍵と照合する。
        let (device_keys, device_json, listed) =
            sender_device_keys(p, sender, sender_device, listed)?;
        if device_keys.curve25519 != self.sender_key {
            return Err(CryptoError::Rejected(Rejection::SenderKeyMismatch));
        }

        // 手順 2：平文の keys.ed25519 が端末の署名鍵と一致する。
        let ed25519 = p
            .get("keys")
            .and_then(|k| k.get("ed25519"))
            .and_then(Value::as_str)
            .ok_or_else(|| CryptoError::InvalidPayload("keys.ed25519 がありません".into()))?;
        if Ed25519PublicKey::from_base64(ed25519)? != device_keys.ed25519 {
            return Err(CryptoError::Rejected(Rejection::SenderSigningKeyMismatch));
        }

        // 手順 3：宛先が自分。
        let recipient_ed25519 = p
            .get("recipient_keys")
            .and_then(|k| k.get("ed25519"))
            .and_then(Value::as_str);
        if p.get("recipient").and_then(Value::as_str) != Some(account.user_id())
            || recipient_ed25519 != Some(&account.identity_keys().ed25519.to_base64())
        {
            return Err(CryptoError::Rejected(Rejection::RecipientMismatch));
        }

        // 手順 4：送信端末を検証済みにできるか。Shape にも keys/query にもない端末はしない。
        let sender_verified = listed
            && trust.is_some_and(|trust| signed_by_self_signing_key(device_json, sender, trust));

        let content = p
            .get("content")
            .ok_or_else(|| CryptoError::InvalidPayload("content がありません".into()))?;
        let room_key = parse_room_key(content)?;
        let session_key = SessionKey::from_base64(&room_key.session_key)
            .map_err(|e| CryptoError::InvalidPayload(format!("session_key が壊れています: {e}")))?;
        let session = InboundGroupSession::new(&session_key, MegolmSessionConfig::version_1());
        if session.session_id() != room_key.session_id {
            return Err(CryptoError::InvalidPayload(
                "session_key から導いた session_id が宣言と違います".into(),
            ));
        }
        Ok(ReceivedRoomKey {
            channel_id: room_key.channel_id,
            epoch: room_key.epoch,
            sender_key: self.sender_key,
            sender_user_id: sender.to_owned(),
            sender_device_id: sender_device.to_owned(),
            sender_signing_key: device_keys.ed25519,
            sender_verified,
            session,
        })
    }
}

/// 手順 1 の、送信端末の端末鍵の決め方。
///
/// 返すのは、検証した端末鍵、手順 4 で署名をたどる端末鍵の JSON、Shape か `keys/query` に
/// 端末があったか。
fn sender_device_keys<'a>(
    plaintext: &'a Value,
    sender: &str,
    sender_device: &str,
    listed: ListedDevice<'a>,
) -> Result<(DeviceKeys, &'a Value, bool), CryptoError> {
    let invalid =
        |e: CryptoError| CryptoError::Rejected(Rejection::InvalidSenderDeviceKeys(e.to_string()));
    let embedded = plaintext.get("sender_device_keys");
    let listed_keys = match listed {
        ListedDevice::Listed(value) => {
            Some((verify_device_keys(value, sender, sender_device)?, value))
        }
        ListedDevice::NotListed => None,
    };
    match (embedded, listed_keys) {
        (Some(embedded), listed_keys) => {
            let keys = verify_device_keys(embedded, sender, sender_device).map_err(invalid)?;
            match listed_keys {
                Some((listed_keys, _)) => {
                    // 端末鍵は登録後に変わらないので、Shape の行と同じ鍵でなければならない。
                    if listed_keys.curve25519 != keys.curve25519
                        || listed_keys.ed25519 != keys.ed25519
                    {
                        return Err(CryptoError::Rejected(Rejection::SenderDeviceKeysConflict));
                    }
                    Ok((keys, embedded, true))
                }
                None => Ok((keys, embedded, false)),
            }
        }
        (None, Some((keys, value))) => Ok((keys, value, true)),
        (None, None) => Err(CryptoError::Rejected(Rejection::UnknownSenderDevice)),
    }
}

/// 端末鍵が、送信ユーザーの自己署名鍵で署名されているか（手順 4）。
fn signed_by_self_signing_key(device_json: &Value, user_id: &str, trust: &SenderTrust) -> bool {
    let key_id = format!("ed25519:{}", trust.self_signing_key.to_base64());
    if verify_json_signature(device_json, user_id, &key_id, &trust.self_signing_key).is_ok() {
        return true;
    }
    let Some(signature) = &trust.signature else {
        return false;
    };
    let mut with_signature = device_json.clone();
    with_signature["signatures"][user_id][&key_id] = Value::String(signature.clone());
    verify_json_signature(&with_signature, user_id, &key_id, &trust.self_signing_key).is_ok()
}

fn parse_room_key(content: &Value) -> Result<RoomKey, CryptoError> {
    if content.get("algorithm").and_then(Value::as_str) != Some(MEGOLM_ALGORITHM) {
        return Err(CryptoError::InvalidPayload(format!(
            "content.algorithm が {MEGOLM_ALGORITHM} ではありません"
        )));
    }
    let str_field = |name: &str| {
        content
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| CryptoError::InvalidPayload(format!("content.{name} がありません")))
    };
    let epoch = content
        .get("epoch")
        .and_then(Value::as_u64)
        .ok_or_else(|| CryptoError::InvalidPayload("content.epoch がありません".into()))?;
    check_safe_integer("content.epoch", epoch)?;
    Ok(RoomKey {
        channel_id: str_field("channel_id")?,
        epoch,
        session_id: str_field("session_id")?,
        session_key: str_field("session_key")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
    const ALICE_DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const ALICE_OTHER_DEVICE: &str = "1b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const BOB: &str = "00000000-0000-0000-0000-00000000000b";
    const BOB_DEVICE: &str = "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const MALLORY: &str = "00000000-0000-0000-0000-00000000000c";
    const MALLORY_DEVICE: &str = "3c8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";

    fn keys(account: &DeviceAccount) -> DeviceKeys {
        verify_device_keys(
            &account.device_keys(),
            account.user_id(),
            account.device_id(),
        )
        .unwrap()
    }

    /// Alice が Bob に送った、正しい zoo.room_key を Bob が復号したもの。
    fn genuine() -> (DeviceAccount, DeviceAccount, OlmDecrypted) {
        let alice = DeviceAccount::new(ALICE, ALICE_DEVICE).unwrap();
        let mut bob = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
        bob.generate_one_time_keys(1);
        let one_time_key = bob.one_time_keys().into_values().next().unwrap();
        let mut session = alice
            .create_outbound_olm_session(&keys(&bob), &one_time_key)
            .unwrap();
        let group = vodozemac::megolm::GroupSession::new(MegolmSessionConfig::version_1());
        let room_key = RoomKey {
            channel_id: "c".into(),
            epoch: 1,
            session_id: group.session_id(),
            session_key: group.session_key().to_base64(),
        };
        let content = alice
            .encrypt_room_key(&mut session, &keys(&bob), &room_key)
            .unwrap();
        let decrypted = bob.decrypt_to_device(&mut Vec::new(), &content).unwrap();
        (alice, bob, decrypted)
    }

    fn tampered(decrypted: &OlmDecrypted, change: impl FnOnce(&mut Value)) -> OlmDecrypted {
        let mut decrypted = decrypted.clone();
        change(&mut decrypted.plaintext);
        decrypted
    }

    fn rejection(result: Result<ReceivedRoomKey, CryptoError>) -> Rejection {
        match result {
            Err(CryptoError::Rejected(reason)) => reason,
            other => panic!("破棄されるはずが {other:?}"),
        }
    }

    #[test]
    fn accepts_genuine_room_keys() {
        let (alice, bob, decrypted) = genuine();
        assert_eq!(decrypted.sender(), Some(ALICE));
        assert_eq!(decrypted.sender_device(), Some(ALICE_DEVICE));
        let listed = alice.device_keys();
        for listed in [ListedDevice::Listed(&listed), ListedDevice::NotListed] {
            let room_key = decrypted.verify_room_key(&bob, listed, None).unwrap();
            assert_eq!(room_key.sender_user_id(), ALICE);
            assert_eq!(room_key.sender_key(), alice.identity_keys().curve25519);
            assert!(!room_key.sender_verified());
        }
    }

    #[test]
    fn rejects_sender_device_keys_of_another_user_or_device() {
        let (_, bob, decrypted) = genuine();
        for other in [
            DeviceAccount::new(MALLORY, MALLORY_DEVICE).unwrap(),
            DeviceAccount::new(ALICE, ALICE_OTHER_DEVICE).unwrap(),
        ] {
            let forged = tampered(&decrypted, |p| {
                p["sender_device_keys"] = other.device_keys()
            });
            assert!(matches!(
                rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
                Rejection::InvalidSenderDeviceKeys(_)
            ));
        }
    }

    #[test]
    fn rejects_sender_device_keys_with_broken_self_signature() {
        let (_, bob, decrypted) = genuine();
        let forged = tampered(&decrypted, |p| {
            p["sender_device_keys"]["algorithms"] = json!([OLM_ALGORITHM]);
        });
        assert!(matches!(
            rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::InvalidSenderDeviceKeys(_)
        ));
    }

    #[test]
    fn rejects_sender_device_keys_that_are_not_the_olm_peer() {
        // Mallory が Alice の正しい端末鍵を sender_device_keys に入れて、自分の Olm セッションで送った。
        let (_, bob, decrypted) = genuine();
        let mallory = DeviceAccount::new(MALLORY, MALLORY_DEVICE).unwrap();
        let mut forged = decrypted.clone();
        forged.sender_key = mallory.identity_keys().curve25519;
        assert_eq!(
            rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::SenderKeyMismatch
        );
    }

    #[test]
    fn rejects_sender_device_keys_that_differ_from_the_shape() {
        let (_, bob, decrypted) = genuine();
        // 同じ ID で別の鍵を持つ端末鍵が Shape にある。
        let listed = DeviceAccount::new(ALICE, ALICE_DEVICE)
            .unwrap()
            .device_keys();
        assert_eq!(
            rejection(decrypted.verify_room_key(&bob, ListedDevice::Listed(&listed), None)),
            Rejection::SenderDeviceKeysConflict
        );
    }

    #[test]
    fn rejects_mismatched_signing_key_and_recipient() {
        let (_, bob, decrypted) = genuine();
        let mallory = DeviceAccount::new(MALLORY, MALLORY_DEVICE).unwrap();
        let mallory_ed25519 = mallory.identity_keys().ed25519.to_base64();

        let forged = tampered(&decrypted, |p| {
            p["keys"]["ed25519"] = json!(mallory_ed25519)
        });
        assert_eq!(
            rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::SenderSigningKeyMismatch
        );
        let forged = tampered(&decrypted, |p| p["recipient"] = json!(MALLORY));
        assert_eq!(
            rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::RecipientMismatch
        );
        let forged = tampered(&decrypted, |p| {
            p["recipient_keys"]["ed25519"] = json!(mallory_ed25519);
        });
        assert_eq!(
            rejection(forged.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::RecipientMismatch
        );
        // 別の端末あてのものを受け取った。
        assert_eq!(
            rejection(decrypted.verify_room_key(&mallory, ListedDevice::NotListed, None)),
            Rejection::RecipientMismatch
        );
    }

    #[test]
    fn needs_some_device_keys() {
        let (alice, bob, decrypted) = genuine();
        let without = tampered(&decrypted, |p| {
            p.as_object_mut().unwrap().remove("sender_device_keys");
        });
        assert_eq!(
            rejection(without.verify_room_key(&bob, ListedDevice::NotListed, None)),
            Rejection::UnknownSenderDevice
        );
        let listed = alice.device_keys();
        without
            .verify_room_key(&bob, ListedDevice::Listed(&listed), None)
            .unwrap();
    }

    #[test]
    fn rejects_malformed_room_keys() {
        let (_, bob, decrypted) = genuine();
        let other_session = vodozemac::megolm::GroupSession::new(MegolmSessionConfig::version_1());
        for change in [
            Box::new(|p: &mut Value| p["type"] = json!("m.room_key")) as Box<dyn Fn(&mut Value)>,
            Box::new(|p: &mut Value| p["content"]["algorithm"] = json!(OLM_ALGORITHM)),
            Box::new(|p: &mut Value| p["content"]["epoch"] = json!("1")),
            Box::new(move |p: &mut Value| {
                p["content"]["session_id"] = json!(other_session.session_id());
            }),
        ] {
            let forged = tampered(&decrypted, change);
            assert!(matches!(
                forged.verify_room_key(&bob, ListedDevice::NotListed, None),
                Err(CryptoError::InvalidPayload(_))
            ));
        }
    }
}
