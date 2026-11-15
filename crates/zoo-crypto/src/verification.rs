//! 他のユーザーの絵文字 SAS 検証（#368。docs/e2ee.md の「絵文字 SAS 検証」）。
//!
//! Matrix の [SAS（`m.sas.v1`）](https://spec.matrix.org/latest/client-server-api/#short-authentication-string-sas-verification)
//! を、Olm で暗号化した to-device メッセージで行う。メッセージの形と名前（`m.key.verification.*`）は
//! Matrix と同じ。zoo では次のように絞る。
//!
//! - 検証できるのは他のユーザーだけ（自分の端末どうしの検証は持たない）。
//! - 要求（`request`）を送った側が `start` を送る。受けた側は `ready` を返すだけ。
//! - 方式は `curve25519-hkdf-sha256`、`sha256`、`hkdf-hmac-sha256.v2`、絵文字（`emoji`）だけ。
//! - 相手の端末は、サーバーの今のマスター鍵から自己署名鍵を経て署名をたどれるものだけ
//!   （[`verification_devices`]）。MAC で確かめるのは、端末の署名鍵とマスター鍵の 2 つ。
//! - 双方が相手の MAC を確かめて `done` を送り合ったら検証済み（[`SasPhase::Done`]）。そのあと、
//!   相手のマスター鍵にユーザー署名鍵で署名する（[`SasVerification::sign_master_key`]）。
//!
//! このモジュールは I/O を持たない。受け取ったメッセージ（[`VerificationMessage`]）を渡すと、
//! 送るメッセージ（[`VerificationOutgoing`]）を返す。暗号化は
//! [`DeviceAccount::encrypt_verification`]、送信は呼び出し側が行う。vodozemac の SAS の状態は
//! 保存できないので、検証はメモリーにだけ置く（閉じたら最初からやり直す）。

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vodozemac::sas::{EstablishedSas, Mac, Sas, SasBytes};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, base64_encode};

use crate::cross_signing::{SignatureCheck, check_signature};
use crate::device::{DeviceAccount, new_uuid_v4, validate_uuid};
use crate::megolm::check_safe_integer;
use crate::trust::{AcceptedMasterKey, KeySnapshot, TrustStore};
use crate::{
    CrossSigningKeyPair, CryptoError, DeviceKeys, KeyUsage, to_canonical_json,
    verify_cross_signing_key, verify_device_keys,
};

/// 検証の要求。
pub const VERIFICATION_REQUEST_TYPE: &str = "m.key.verification.request";
/// 要求を受け入れた。
pub const VERIFICATION_READY_TYPE: &str = "m.key.verification.ready";
/// SAS を始める（要求を送った側が送る）。
pub const VERIFICATION_START_TYPE: &str = "m.key.verification.start";
/// `start` を受け入れた（コミットメントを含む）。
pub const VERIFICATION_ACCEPT_TYPE: &str = "m.key.verification.accept";
/// SAS の一時的な公開鍵。
pub const VERIFICATION_KEY_TYPE: &str = "m.key.verification.key";
/// 端末の署名鍵とマスター鍵の MAC。
pub const VERIFICATION_MAC_TYPE: &str = "m.key.verification.mac";
/// 相手の MAC を確かめた。
pub const VERIFICATION_DONE_TYPE: &str = "m.key.verification.done";
/// 取りやめた。
pub const VERIFICATION_CANCEL_TYPE: &str = "m.key.verification.cancel";

/// Olm で暗号化して送る、SAS の検証のメッセージの `type`（docs/e2ee.md の「Olm（to-device メッセージ）」）。
pub const VERIFICATION_TYPES: [&str; 8] = [
    VERIFICATION_REQUEST_TYPE,
    VERIFICATION_READY_TYPE,
    VERIFICATION_START_TYPE,
    VERIFICATION_ACCEPT_TYPE,
    VERIFICATION_KEY_TYPE,
    VERIFICATION_MAC_TYPE,
    VERIFICATION_DONE_TYPE,
    VERIFICATION_CANCEL_TYPE,
];

/// 検証の方式。
pub const SAS_METHOD: &str = "m.sas.v1";
const KEY_AGREEMENT_PROTOCOL: &str = "curve25519-hkdf-sha256";
const HASH: &str = "sha256";
const MAC_METHOD: &str = "hkdf-hmac-sha256.v2";
const SAS_EMOJI: &str = "emoji";

/// 要求から、この時間（ミリ秒）がたっても終わらない検証は取りやめる（`m.timeout`）。
/// Matrix の推奨と同じ 10 分。
pub const VERIFICATION_TIMEOUT_MS: u64 = 10 * 60 * 1000;
/// 要求の `timestamp` がこれ（ミリ秒）より先の時刻なら受け付けない（Matrix と同じ 5 分）。
pub const VERIFICATION_MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

/// `m.key.verification.cancel` の `code`。
pub mod cancel_code {
    /// 利用者が取りやめた、または要求を断った。
    pub const USER: &str = "m.user";
    /// 時間内に終わらなかった。
    pub const TIMEOUT: &str = "m.timeout";
    /// 対応していない方式。
    pub const UNKNOWN_METHOD: &str = "m.unknown_method";
    /// 今の状態で受け付けないメッセージ。
    pub const UNEXPECTED_MESSAGE: &str = "m.unexpected_message";
    /// MAC が合わない、または知らない鍵の MAC。
    pub const KEY_MISMATCH: &str = "m.key_mismatch";
    /// 形式が正しくない。
    pub const INVALID_MESSAGE: &str = "m.invalid_message";
    /// 相手の別の端末が要求を受け入れた。
    pub const ACCEPTED: &str = "m.accepted";
    /// 相手の公開鍵がコミットメントと合わない。
    pub const MISMATCHED_COMMITMENT: &str = "m.mismatched_commitment";
    /// 利用者が、絵文字が一致しないと答えた。
    pub const MISMATCHED_SAS: &str = "m.mismatched_sas";
}

/// 受信時の検証（手順 1〜3）を通った、SAS の検証のメッセージ
/// （[`OlmDecrypted::verify_verification`](crate::OlmDecrypted::verify_verification)）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationMessage {
    /// 送信ユーザー。
    pub sender_user_id: String,
    /// 送信端末の端末鍵（Olm セッションの相手と照合済み）。
    pub sender_device: DeviceKeys,
    /// `m.key.verification.*`。
    pub event_type: String,
    /// `content`（JSON オブジェクト）。
    pub content: Value,
}

impl VerificationMessage {
    /// `content.transaction_id`。
    pub fn transaction_id(&self) -> Option<&str> {
        self.content.get("transaction_id").and_then(Value::as_str)
    }
}

/// 送るメッセージ。`devices` のそれぞれに、[`DeviceAccount::encrypt_verification`] で暗号化して送る。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationOutgoing {
    /// 宛先の端末。
    pub devices: Vec<DeviceKeys>,
    /// `m.key.verification.*`。
    pub event_type: &'static str,
    /// `content`。
    pub content: Value,
}

/// 利用者に見せる検証の段階。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasPhase {
    /// 要求を送り、相手が受け入れるのを待っている。
    Requested,
    /// 相手から要求を受けた。受け入れるか断るかを利用者に聞く。
    Incoming,
    /// 受け入れた。鍵を交換している。
    Exchanging,
    /// 絵文字を出し、一致するかを利用者に聞く。
    Compare,
    /// 一致すると答えた。相手の確認を待っている。
    Confirmed,
    /// 双方が確かめた。検証済み。
    Done,
    /// 取りやめた（[`SasVerification::cancellation`]）。
    Cancelled,
}

/// 取りやめた理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasCancellation {
    /// `m.key.verification.cancel` の `code`（[`cancel_code`]）。
    pub code: String,
    /// この端末が取りやめたか（`false` なら相手から届いた）。
    pub by_us: bool,
}

/// 自分の端末と、MAC で相手に示す自分の鍵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationIdentity {
    user_id: String,
    device_id: String,
    device_ed25519: Ed25519PublicKey,
    master_key: Ed25519PublicKey,
}

impl VerificationIdentity {
    /// 今の端末と、記録で信頼した自分のマスター鍵から作る。自分のマスター鍵をまだ信頼していない
    /// （解錠の前）ときは [`CryptoError::Verification`]。
    pub fn new(account: &DeviceAccount, trust: &TrustStore) -> Result<Self, CryptoError> {
        if trust.user_id() != account.user_id() {
            return Err(CryptoError::Verification(
                "配布先の判定の記録が今の端末のユーザーのものではありません".into(),
            ));
        }
        let master = trust.own_master_key().ok_or_else(|| {
            CryptoError::Verification("自分のマスター鍵をまだ確かめていません".into())
        })?;
        Ok(Self {
            user_id: account.user_id().to_owned(),
            device_id: account.device_id().to_owned(),
            device_ed25519: account.identity_keys().ed25519,
            master_key: Ed25519PublicKey::from_base64(master)?,
        })
    }
}

/// `user_id` の、サーバーの今のマスター鍵から自己署名鍵を経て署名をたどれる端末と、そのマスター鍵。
///
/// `snapshot` は `keys/query` の応答（相手のマスター鍵、自己署名鍵、端末鍵、署名）。マスター鍵を
/// 信頼するかどうかは問わない（それを確かめるのが SAS）。マスター鍵か自己署名鍵がない、壊れている
/// ときは [`CryptoError::Verification`]。
pub fn verification_devices(
    snapshot: &KeySnapshot,
    user_id: &str,
) -> Result<(Ed25519PublicKey, Vec<DeviceKeys>), CryptoError> {
    let no_keys =
        || CryptoError::Verification(format!("{user_id} はクロス署名鍵を設定していません"));
    let master_json = snapshot.master_keys.get(user_id).ok_or_else(no_keys)?;
    let master = verify_cross_signing_key(master_json, user_id, KeyUsage::Master)?;
    let ssk_json = snapshot
        .self_signing_keys
        .get(user_id)
        .ok_or_else(no_keys)?;
    let ssk = verify_cross_signing_key(ssk_json, user_id, KeyUsage::SelfSigning)?;
    if check_signature(
        ssk_json,
        user_id,
        &ssk.to_base64(),
        user_id,
        &master,
        &snapshot.signatures,
    ) != SignatureCheck::Valid
    {
        return Err(CryptoError::Verification(format!(
            "{user_id} の自己署名鍵がマスター鍵で署名されていません"
        )));
    }
    let mut devices = Vec::new();
    for (device_id, json) in snapshot.device_keys.get(user_id).into_iter().flatten() {
        let Ok(keys) = verify_device_keys(json, user_id, device_id) else {
            continue;
        };
        if check_signature(
            json,
            user_id,
            device_id,
            user_id,
            &ssk,
            &snapshot.signatures,
        ) == SignatureCheck::Valid
        {
            devices.push(keys);
        }
    }
    Ok((master, devices))
}

/// 相手の端末と、その端末のユーザーのマスター鍵。
#[derive(Debug, Clone)]
struct Peer {
    device: DeviceKeys,
    master_key: Ed25519PublicKey,
}

impl Peer {
    /// `snapshot` で `device` が相手のマスター鍵からたどれることを確かめる。
    fn from_snapshot(snapshot: &KeySnapshot, device: &DeviceKeys) -> Option<Self> {
        let (master_key, devices) = verification_devices(snapshot, &device.user_id).ok()?;
        devices
            .into_iter()
            .find(|d| d == device)
            .map(|device| Self { device, master_key })
    }
}

/// 絵文字を見せている段階の状態。
struct Compare {
    established: EstablishedSas,
    bytes: SasBytes,
    /// 利用者が一致すると答え、MAC を送った。
    confirmed: bool,
    /// 届いた相手の MAC（確かめる前）。
    their_mac: Option<Value>,
    /// 相手の MAC を確かめ、`done` を送った。
    mac_verified: bool,
    /// 相手の `done` が届いた。
    their_done: bool,
}

enum State {
    Requested,
    Incoming,
    Ready,
    /// 要求した側が `start` を送り、`accept` を待っている。
    Started {
        sas: Sas,
        start: Value,
    },
    /// 受けた側が `accept` を送り、相手の `key` を待っている。
    Accepted {
        sas: Sas,
    },
    /// 要求した側が `key` を送り、相手の `key` を待っている。
    KeySent {
        sas: Sas,
        start: Value,
        commitment: String,
    },
    Compare(Box<Compare>),
    Done,
    Cancelled(SasCancellation),
    /// 状態を移す途中（外からは見えない）。
    Transition,
}

/// 1 つの SAS の検証（1 つの `transaction_id`）。
pub struct SasVerification {
    txn_id: String,
    own: VerificationIdentity,
    their_user_id: String,
    we_requested: bool,
    /// 要求を送った相手の端末（要求した側だけ）。
    requested_devices: Vec<DeviceKeys>,
    /// 検証の相手の端末（要求した側は `ready` を受けてから決まる）。
    peer: Option<Peer>,
    started_at_ms: u64,
    state: State,
}

impl std::fmt::Debug for SasVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SasVerification")
            .field("txn_id", &self.txn_id)
            .field("their_user_id", &self.their_user_id)
            .field("we_requested", &self.we_requested)
            .field("phase", &self.phase())
            .finish_non_exhaustive()
    }
}

impl SasVerification {
    /// `their_user_id` に検証を求める。要求は、[`verification_devices`] のすべての端末に送る。
    ///
    /// `snapshot` は相手の `keys/query` の応答。`now_ms` は今の時刻（Unix 時間のミリ秒）。
    pub fn request(
        own: VerificationIdentity,
        snapshot: &KeySnapshot,
        their_user_id: &str,
        now_ms: u64,
    ) -> Result<(Self, VerificationOutgoing), CryptoError> {
        validate_uuid("user_id", their_user_id, false)?;
        check_safe_integer("now_ms", now_ms)?;
        if their_user_id == own.user_id {
            return Err(CryptoError::Verification(
                "自分自身は SAS で検証できません".into(),
            ));
        }
        let (_, devices) = verification_devices(snapshot, their_user_id)?;
        if devices.is_empty() {
            return Err(CryptoError::Verification(format!(
                "{their_user_id} に検証できる端末がありません"
            )));
        }
        let txn_id = new_uuid_v4();
        let outgoing = VerificationOutgoing {
            devices: devices.clone(),
            event_type: VERIFICATION_REQUEST_TYPE,
            content: json!({
                "from_device": own.device_id,
                "methods": [SAS_METHOD],
                "timestamp": now_ms,
                "transaction_id": txn_id,
            }),
        };
        Ok((
            Self {
                txn_id,
                own,
                their_user_id: their_user_id.to_owned(),
                we_requested: true,
                requested_devices: devices,
                peer: None,
                started_at_ms: now_ms,
                state: State::Requested,
            },
            outgoing,
        ))
    }

    /// 届いた要求（`m.key.verification.request`）から検証を作る。利用者が受け入れるまで何も送らない
    /// （[`accept`](Self::accept)）。
    ///
    /// 次のときは [`CryptoError::Verification`] か [`CryptoError::InvalidPayload`] を返し、
    /// 呼び出し側は要求を捨てる（取りやめも送らない）。
    ///
    /// - 自分からの要求、`m.sas.v1` を含まない要求
    /// - `timestamp` が 10 分より前か 5 分より先（古い要求の再生を受け付けない）
    /// - 送信端末が、`snapshot` の相手のマスター鍵からたどれない
    pub fn from_request(
        own: VerificationIdentity,
        message: &VerificationMessage,
        snapshot: &KeySnapshot,
        now_ms: u64,
    ) -> Result<Self, CryptoError> {
        check_safe_integer("now_ms", now_ms)?;
        if message.event_type != VERIFICATION_REQUEST_TYPE {
            return Err(CryptoError::InvalidPayload(format!(
                "type が {VERIFICATION_REQUEST_TYPE} ではありません"
            )));
        }
        let content = &message.content;
        let txn_id = transaction_id(content)?;
        if message.sender_user_id == own.user_id {
            return Err(CryptoError::Verification(
                "自分の端末からの要求は受け付けません".into(),
            ));
        }
        if content.get("from_device").and_then(Value::as_str)
            != Some(message.sender_device.device_id.as_str())
        {
            return Err(CryptoError::InvalidPayload(
                "from_device が送信端末ではありません".into(),
            ));
        }
        if !string_list(content, "methods").contains(&SAS_METHOD) {
            return Err(CryptoError::Verification(format!(
                "{SAS_METHOD} に対応していない要求です"
            )));
        }
        let timestamp = content
            .get("timestamp")
            .and_then(Value::as_u64)
            .ok_or_else(|| CryptoError::InvalidPayload("timestamp がありません".into()))?;
        if timestamp.saturating_add(VERIFICATION_TIMEOUT_MS) < now_ms
            || timestamp > now_ms.saturating_add(VERIFICATION_MAX_CLOCK_SKEW_MS)
        {
            return Err(CryptoError::Verification(
                "要求の時刻が古すぎるか、先すぎます".into(),
            ));
        }
        let peer = Peer::from_snapshot(snapshot, &message.sender_device).ok_or_else(|| {
            CryptoError::Verification("送信端末を相手のマスター鍵からたどれません".into())
        })?;
        Ok(Self {
            txn_id: txn_id.to_owned(),
            own,
            their_user_id: message.sender_user_id.clone(),
            we_requested: false,
            requested_devices: Vec::new(),
            peer: Some(peer),
            // 期限は受け取った時刻ではなく、要求の時刻から数える（Matrix と同じく、要求の
            // `timestamp` から 10 分で閉じる）。少し先の時刻の要求は、受け取った時刻から数える。
            started_at_ms: timestamp.min(now_ms),
            state: State::Incoming,
        })
    }

    /// `transaction_id`。
    pub fn transaction_id(&self) -> &str {
        &self.txn_id
    }

    /// 相手のユーザー。
    pub fn their_user_id(&self) -> &str {
        &self.their_user_id
    }

    /// 相手の端末（要求した側は、相手が受け入れるまで `None`）。
    pub fn their_device_id(&self) -> Option<&str> {
        self.peer.as_ref().map(|p| p.device.device_id.as_str())
    }

    /// この端末が要求したか。
    pub fn we_requested(&self) -> bool {
        self.we_requested
    }

    /// 期限を数え始める時刻（Unix 時間のミリ秒）。要求した側は要求した時刻、受けた側は要求の `timestamp`。
    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    /// 利用者に見せる段階。
    pub fn phase(&self) -> SasPhase {
        match &self.state {
            State::Requested => SasPhase::Requested,
            State::Incoming => SasPhase::Incoming,
            State::Ready
            | State::Started { .. }
            | State::Accepted { .. }
            | State::KeySent { .. }
            | State::Transition => SasPhase::Exchanging,
            State::Compare(c) if c.confirmed => SasPhase::Confirmed,
            State::Compare(_) => SasPhase::Compare,
            State::Done => SasPhase::Done,
            State::Cancelled(_) => SasPhase::Cancelled,
        }
    }

    /// 絵文字の番号（Matrix の SAS の絵文字の表の 0〜63）を 7 つ。絵文字を見せる段階でなければ `None`。
    pub fn emoji_indices(&self) -> Option<[u8; 7]> {
        match &self.state {
            State::Compare(c) => Some(c.bytes.emoji_indices()),
            _ => None,
        }
    }

    /// 取りやめた理由。取りやめていなければ `None`。
    pub fn cancellation(&self) -> Option<&SasCancellation> {
        match &self.state {
            State::Cancelled(c) => Some(c),
            _ => None,
        }
    }

    /// 終わった（検証済みか、取りやめた）。
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done | State::Cancelled(_))
    }

    /// 検証できた相手のマスター鍵（[`SasPhase::Done`] のときだけ）。
    pub fn verified_master_key(&self) -> Option<Ed25519PublicKey> {
        match (&self.state, &self.peer) {
            (State::Done, Some(peer)) => Some(peer.master_key),
            _ => None,
        }
    }

    /// 利用者が要求を受け入れた。`ready` を返す。
    pub fn accept(&mut self) -> Result<Vec<VerificationOutgoing>, CryptoError> {
        if !matches!(self.state, State::Incoming) {
            return Err(CryptoError::Verification(
                "受け入れられるのは届いた要求だけです".into(),
            ));
        }
        self.state = State::Ready;
        Ok(vec![self.to_peer(
            VERIFICATION_READY_TYPE,
            json!({
                "from_device": self.own.device_id,
                "methods": [SAS_METHOD],
                "transaction_id": self.txn_id,
            }),
        )])
    }

    /// 利用者が、絵文字が一致すると答えた。自分の MAC を送る。相手の MAC が先に届いていれば、
    /// それを確かめて `done` も送る。
    pub fn confirm(&mut self) -> Result<Vec<VerificationOutgoing>, CryptoError> {
        let State::Compare(compare) = &mut self.state else {
            return Err(CryptoError::Verification(
                "絵文字を見せる段階ではありません".into(),
            ));
        };
        if compare.confirmed {
            return Ok(Vec::new());
        }
        compare.confirmed = true;
        let mac = own_mac(
            &self.own,
            &self.their_user_id,
            self.peer.as_ref(),
            &self.txn_id,
            compare,
        )?;
        let mut out = vec![self.to_peer(VERIFICATION_MAC_TYPE, mac)];
        out.extend(self.check_their_mac());
        Ok(out)
    }

    /// 利用者が、絵文字が一致しないと答えた（`m.mismatched_sas` で取りやめる）。
    pub fn mismatch(&mut self) -> Vec<VerificationOutgoing> {
        self.cancel_with(cancel_code::MISMATCHED_SAS)
    }

    /// 利用者が取りやめた、または要求を断った（`m.user`）。終わった検証では何もしない。
    pub fn cancel(&mut self) -> Vec<VerificationOutgoing> {
        self.cancel_with(cancel_code::USER)
    }

    /// 要求から [`VERIFICATION_TIMEOUT_MS`] たっても終わっていなければ取りやめる（`m.timeout`）。
    pub fn expire(&mut self, now_ms: u64) -> Vec<VerificationOutgoing> {
        if self.is_finished() || now_ms < self.started_at_ms.saturating_add(VERIFICATION_TIMEOUT_MS)
        {
            return Vec::new();
        }
        self.cancel_with(cancel_code::TIMEOUT)
    }

    /// 届いたメッセージで状態を進め、送るメッセージを返す。
    ///
    /// - `transaction_id` が違う、相手のユーザーや端末でない送信者からのメッセージは無視する。
    /// - 今の段階で受け付けないメッセージや、形式が正しくないメッセージなら、取りやめを送る。
    /// - `snapshot` は送信ユーザーの `keys/query` の応答。要求した側が `ready` を受けたときに、
    ///   送信端末が相手のマスター鍵からたどれるかを確かめる。
    pub fn receive(
        &mut self,
        message: &VerificationMessage,
        snapshot: &KeySnapshot,
    ) -> Vec<VerificationOutgoing> {
        if self.is_finished()
            || message.transaction_id() != Some(self.txn_id.as_str())
            || message.sender_user_id != self.their_user_id
        {
            return Vec::new();
        }
        let from_peer = self
            .peer
            .as_ref()
            .is_some_and(|peer| peer.device == message.sender_device);
        let content = &message.content;
        match message.event_type.as_str() {
            VERIFICATION_CANCEL_TYPE => {
                // 相手の端末（要求した側は、要求を送ったどれか）からの取りやめだけを受け付ける。
                let from_requested =
                    self.peer.is_none() && self.requested_devices.contains(&message.sender_device);
                if from_peer || from_requested {
                    let code = content
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or(cancel_code::USER)
                        .chars()
                        .take(255)
                        .collect();
                    self.state = State::Cancelled(SasCancellation { code, by_us: false });
                }
                Vec::new()
            }
            VERIFICATION_READY_TYPE if self.peer.is_none() => self.on_ready(message, snapshot),
            // 相手の別の端末が、あとから受け入れた。その端末には取りやめを送る。
            VERIFICATION_READY_TYPE if !from_peer && self.we_requested => {
                if self.requested_devices.contains(&message.sender_device) {
                    vec![
                        self.cancel_message(
                            vec![message.sender_device.clone()],
                            cancel_code::ACCEPTED,
                        ),
                    ]
                } else {
                    Vec::new()
                }
            }
            _ if !from_peer => Vec::new(),
            VERIFICATION_START_TYPE => self.on_start(content),
            VERIFICATION_ACCEPT_TYPE => self.on_accept(content),
            VERIFICATION_KEY_TYPE => self.on_key(content),
            VERIFICATION_MAC_TYPE => self.on_mac(content),
            VERIFICATION_DONE_TYPE => self.on_done(),
            _ => self.cancel_with(cancel_code::UNEXPECTED_MESSAGE),
        }
    }

    /// 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する
    /// （[`TrustStore::accept_master_key`] と同じく、`replace: true` のピンと署名を返し、記録を
    /// 登録待ちで置き換える）。
    ///
    /// `master_key` は、署名する直前に `keys/query` で読んだ相手のマスター鍵の JSON。SAS で確かめた
    /// マスター鍵と違えば（検証のあとに作り直された）[`CryptoError::Verification`]。
    pub fn sign_master_key(
        &self,
        trust: &mut TrustStore,
        user_signing: &CrossSigningKeyPair,
        master_key: &Value,
    ) -> Result<AcceptedMasterKey, CryptoError> {
        let verified = self
            .verified_master_key()
            .ok_or_else(|| CryptoError::Verification("検証が終わっていません".into()))?;
        let current = verify_cross_signing_key(master_key, &self.their_user_id, KeyUsage::Master)?;
        if current != verified {
            return Err(CryptoError::Verification(
                "検証したあとに相手のマスター鍵が変わりました".into(),
            ));
        }
        trust.accept_master_key(user_signing, &self.their_user_id, master_key)
    }

    fn on_ready(
        &mut self,
        message: &VerificationMessage,
        snapshot: &KeySnapshot,
    ) -> Vec<VerificationOutgoing> {
        if !self.we_requested || !matches!(self.state, State::Requested) {
            return Vec::new();
        }
        if !self.requested_devices.contains(&message.sender_device) {
            return Vec::new();
        }
        let content = &message.content;
        if content.get("from_device").and_then(Value::as_str)
            != Some(message.sender_device.device_id.as_str())
        {
            return Vec::new();
        }
        // 要求のあとで鍵が変わり、たどれなくなった端末は相手にしない。
        let Some(peer) = Peer::from_snapshot(snapshot, &message.sender_device) else {
            return vec![self.cancel_message(
                vec![message.sender_device.clone()],
                cancel_code::KEY_MISMATCH,
            )];
        };
        self.peer = Some(peer);
        if !string_list(content, "methods").contains(&SAS_METHOD) {
            return self.cancel_with(cancel_code::UNKNOWN_METHOD);
        }
        let sas = Sas::new();
        let start = json!({
            "from_device": self.own.device_id,
            "method": SAS_METHOD,
            "key_agreement_protocols": [KEY_AGREEMENT_PROTOCOL],
            "hashes": [HASH],
            "message_authentication_codes": [MAC_METHOD],
            "short_authentication_string": [SAS_EMOJI],
            "transaction_id": self.txn_id,
        });
        self.state = State::Started {
            sas,
            start: start.clone(),
        };
        let mut out = vec![self.to_peer(VERIFICATION_START_TYPE, start)];
        // 要求を送ったほかの端末には、別の端末が受け入れたことを知らせる。
        let others: Vec<DeviceKeys> = self
            .requested_devices
            .iter()
            .filter(|d| **d != message.sender_device)
            .cloned()
            .collect();
        if !others.is_empty() {
            out.push(self.cancel_message(others, cancel_code::ACCEPTED));
        }
        out
    }

    fn on_start(&mut self, content: &Value) -> Vec<VerificationOutgoing> {
        if self.we_requested || !matches!(self.state, State::Ready) {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        }
        let peer_device = self.peer.as_ref().map(|p| p.device.device_id.as_str());
        if content.get("from_device").and_then(Value::as_str) != peer_device {
            return self.cancel_with(cancel_code::INVALID_MESSAGE);
        }
        if content.get("method").and_then(Value::as_str) != Some(SAS_METHOD)
            || !string_list(content, "key_agreement_protocols").contains(&KEY_AGREEMENT_PROTOCOL)
            || !string_list(content, "hashes").contains(&HASH)
            || !string_list(content, "message_authentication_codes").contains(&MAC_METHOD)
            || !string_list(content, "short_authentication_string").contains(&SAS_EMOJI)
        {
            return self.cancel_with(cancel_code::UNKNOWN_METHOD);
        }
        let Ok(canonical) = to_canonical_json(content) else {
            return self.cancel_with(cancel_code::INVALID_MESSAGE);
        };
        let sas = Sas::new();
        let commitment = commitment(&sas.public_key(), &canonical);
        self.state = State::Accepted { sas };
        vec![self.to_peer(
            VERIFICATION_ACCEPT_TYPE,
            json!({
                "method": SAS_METHOD,
                "key_agreement_protocol": KEY_AGREEMENT_PROTOCOL,
                "hash": HASH,
                "message_authentication_code": MAC_METHOD,
                "short_authentication_string": [SAS_EMOJI],
                "commitment": commitment,
                "transaction_id": self.txn_id,
            }),
        )]
    }

    fn on_accept(&mut self, content: &Value) -> Vec<VerificationOutgoing> {
        if !matches!(self.state, State::Started { .. }) {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        }
        let field = |name: &str| content.get(name).and_then(Value::as_str);
        if field("key_agreement_protocol") != Some(KEY_AGREEMENT_PROTOCOL)
            || field("hash") != Some(HASH)
            || field("message_authentication_code") != Some(MAC_METHOD)
            || !string_list(content, "short_authentication_string").contains(&SAS_EMOJI)
        {
            return self.cancel_with(cancel_code::UNKNOWN_METHOD);
        }
        let Some(commitment) = field("commitment").map(str::to_owned) else {
            return self.cancel_with(cancel_code::INVALID_MESSAGE);
        };
        let State::Started { sas, start } = std::mem::replace(&mut self.state, State::Transition)
        else {
            unreachable!("上で確かめた");
        };
        let key = sas.public_key().to_base64();
        self.state = State::KeySent {
            sas,
            start,
            commitment,
        };
        vec![self.to_peer(
            VERIFICATION_KEY_TYPE,
            json!({ "key": key, "transaction_id": self.txn_id }),
        )]
    }

    fn on_key(&mut self, content: &Value) -> Vec<VerificationOutgoing> {
        if !matches!(self.state, State::Accepted { .. } | State::KeySent { .. }) {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        }
        let Some(their_key) = content
            .get("key")
            .and_then(Value::as_str)
            .and_then(|k| Curve25519PublicKey::from_base64(k).ok())
        else {
            return self.cancel_with(cancel_code::INVALID_MESSAGE);
        };
        let mut out = Vec::new();
        let (sas, starter_key, accepter_key) =
            match std::mem::replace(&mut self.state, State::Transition) {
                // 要求した側：相手の鍵がコミットメントと合うかを確かめる。
                State::KeySent {
                    sas,
                    start,
                    commitment: expected,
                } => {
                    let canonical = to_canonical_json(&start).unwrap_or_default();
                    if commitment(&their_key, &canonical) != expected {
                        return self.cancel_with(cancel_code::MISMATCHED_COMMITMENT);
                    }
                    let own = sas.public_key();
                    (sas, own, their_key)
                }
                // 受けた側：自分の鍵を返す。
                State::Accepted { sas } => {
                    let own = sas.public_key();
                    out.push(self.to_peer(
                        VERIFICATION_KEY_TYPE,
                        json!({ "key": own.to_base64(), "transaction_id": self.txn_id }),
                    ));
                    (sas, their_key, own)
                }
                _ => unreachable!("上で確かめた"),
            };
        let Ok(established) = sas.diffie_hellman(their_key) else {
            return self.cancel_with(cancel_code::INVALID_MESSAGE);
        };
        let info = self.sas_info(&starter_key, &accepter_key);
        let bytes = established.bytes(&info);
        self.state = State::Compare(Box::new(Compare {
            established,
            bytes,
            confirmed: false,
            their_mac: None,
            mac_verified: false,
            their_done: false,
        }));
        out
    }

    fn on_mac(&mut self, content: &Value) -> Vec<VerificationOutgoing> {
        let State::Compare(compare) = &mut self.state else {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        };
        if compare.their_mac.is_some() {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        }
        compare.their_mac = Some(content.clone());
        self.check_their_mac()
    }

    fn on_done(&mut self) -> Vec<VerificationOutgoing> {
        let State::Compare(compare) = &mut self.state else {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        };
        compare.their_done = true;
        if compare.mac_verified {
            self.state = State::Done;
        }
        Vec::new()
    }

    /// 自分が一致すると答えていて、相手の MAC が届いていれば確かめる。合えば `done` を送る。
    fn check_their_mac(&mut self) -> Vec<VerificationOutgoing> {
        let State::Compare(compare) = &mut self.state else {
            return Vec::new();
        };
        if !compare.confirmed || compare.mac_verified {
            return Vec::new();
        }
        let Some(mac) = compare.their_mac.take() else {
            return Vec::new();
        };
        let Some(peer) = &self.peer else {
            return self.cancel_with(cancel_code::UNEXPECTED_MESSAGE);
        };
        if verify_their_mac(&self.own, peer, &self.txn_id, &compare.established, &mac).is_err() {
            return self.cancel_with(cancel_code::KEY_MISMATCH);
        }
        compare.mac_verified = true;
        let their_done = compare.their_done;
        let done = self.to_peer(
            VERIFICATION_DONE_TYPE,
            json!({ "transaction_id": self.txn_id }),
        );
        if their_done {
            self.state = State::Done;
        }
        vec![done]
    }

    /// `MATRIX_KEY_VERIFICATION_SAS|<start を送ったユーザー>|<端末>|<鍵>|<accept を送ったユーザー>|<端末>|<鍵>|<transaction_id>`。
    fn sas_info(
        &self,
        starter_key: &Curve25519PublicKey,
        accepter_key: &Curve25519PublicKey,
    ) -> String {
        let their_device = self
            .peer
            .as_ref()
            .map(|p| p.device.device_id.as_str())
            .unwrap_or_default();
        let (starter, accepter) = if self.we_requested {
            (
                (self.own.user_id.as_str(), self.own.device_id.as_str()),
                (self.their_user_id.as_str(), their_device),
            )
        } else {
            (
                (self.their_user_id.as_str(), their_device),
                (self.own.user_id.as_str(), self.own.device_id.as_str()),
            )
        };
        format!(
            "MATRIX_KEY_VERIFICATION_SAS|{}|{}|{}|{}|{}|{}|{}",
            starter.0,
            starter.1,
            starter_key.to_base64(),
            accepter.0,
            accepter.1,
            accepter_key.to_base64(),
            self.txn_id
        )
    }

    fn to_peer(&self, event_type: &'static str, content: Value) -> VerificationOutgoing {
        VerificationOutgoing {
            devices: self.peer.iter().map(|p| p.device.clone()).collect(),
            event_type,
            content,
        }
    }

    fn cancel_message(&self, devices: Vec<DeviceKeys>, code: &str) -> VerificationOutgoing {
        VerificationOutgoing {
            devices,
            event_type: VERIFICATION_CANCEL_TYPE,
            content: json!({
                "code": code,
                "reason": cancel_reason(code),
                "transaction_id": self.txn_id,
            }),
        }
    }

    /// 取りやめる。相手の端末（決まっていなければ要求を送ったすべての端末）に取りやめを送る。
    fn cancel_with(&mut self, code: &str) -> Vec<VerificationOutgoing> {
        if self.is_finished() {
            return Vec::new();
        }
        self.state = State::Cancelled(SasCancellation {
            code: code.to_owned(),
            by_us: true,
        });
        let devices = match &self.peer {
            Some(peer) => vec![peer.device.clone()],
            None => self.requested_devices.clone(),
        };
        if devices.is_empty() {
            return Vec::new();
        }
        vec![self.cancel_message(devices, code)]
    }
}

fn transaction_id(content: &Value) -> Result<&str, CryptoError> {
    let txn_id = content
        .get("transaction_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CryptoError::InvalidPayload("transaction_id がありません".into()))?;
    validate_uuid("transaction_id", txn_id, false)?;
    Ok(txn_id)
}

fn string_list<'a>(content: &'a Value, name: &str) -> Vec<&'a str> {
    content
        .get(name)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// `accept` のコミットメント：`base64(SHA-256(<公開鍵の base64> || <start の Canonical JSON>))`。
fn commitment(public_key: &Curve25519PublicKey, start_canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key.to_base64().as_bytes());
    hasher.update(start_canonical.as_bytes());
    base64_encode(hasher.finalize())
}

fn cancel_reason(code: &str) -> &'static str {
    match code {
        cancel_code::USER => "The user cancelled the verification",
        cancel_code::TIMEOUT => "The verification timed out",
        cancel_code::UNKNOWN_METHOD => "Unsupported verification method",
        cancel_code::KEY_MISMATCH => "The keys did not match",
        cancel_code::INVALID_MESSAGE => "Invalid message",
        cancel_code::ACCEPTED => "Another device accepted the request",
        cancel_code::MISMATCHED_COMMITMENT => "The commitment did not match",
        cancel_code::MISMATCHED_SAS => "The emoji did not match",
        _ => "Unexpected message",
    }
}

/// MAC の `info` の共通部分：`MATRIX_KEY_VERIFICATION_MAC<送るユーザー><端末><受けるユーザー><端末><transaction_id>`。
fn mac_info_base(
    sender_user: &str,
    sender_device: &str,
    receiver_user: &str,
    receiver_device: &str,
    txn_id: &str,
) -> String {
    format!(
        "MATRIX_KEY_VERIFICATION_MAC{sender_user}{sender_device}{receiver_user}{receiver_device}{txn_id}"
    )
}

/// 自分の端末の署名鍵とマスター鍵の MAC（`m.key.verification.mac` の `content`）。
fn own_mac(
    own: &VerificationIdentity,
    their_user_id: &str,
    peer: Option<&Peer>,
    txn_id: &str,
    compare: &Compare,
) -> Result<Value, CryptoError> {
    let peer =
        peer.ok_or_else(|| CryptoError::Verification("相手の端末が決まっていません".into()))?;
    let base = mac_info_base(
        &own.user_id,
        &own.device_id,
        their_user_id,
        &peer.device.device_id,
        txn_id,
    );
    let keys = [
        (
            format!("ed25519:{}", own.device_id),
            own.device_ed25519.to_base64(),
        ),
        (
            format!("ed25519:{}", own.master_key.to_base64()),
            own.master_key.to_base64(),
        ),
    ];
    let mut mac = Map::new();
    for (key_id, key) in &keys {
        let tag = compare
            .established
            .calculate_mac(key, &format!("{base}{key_id}"));
        mac.insert(key_id.clone(), Value::String(tag.to_base64()));
    }
    let mut ids: Vec<&str> = keys.iter().map(|(id, _)| id.as_str()).collect();
    ids.sort_unstable();
    let key_ids = compare
        .established
        .calculate_mac(&ids.join(","), &format!("{base}KEY_IDS"));
    Ok(json!({
        "mac": mac,
        "keys": key_ids.to_base64(),
        "transaction_id": txn_id,
    }))
}

/// 相手の MAC を確かめる。端末の署名鍵とマスター鍵のほかの鍵の MAC や、マスター鍵の MAC がない
/// ものは受け付けない。
fn verify_their_mac(
    own: &VerificationIdentity,
    peer: &Peer,
    txn_id: &str,
    established: &EstablishedSas,
    content: &Value,
) -> Result<(), ()> {
    let base = mac_info_base(
        &peer.device.user_id,
        &peer.device.device_id,
        &own.user_id,
        &own.device_id,
        txn_id,
    );
    let mac = content.get("mac").and_then(Value::as_object).ok_or(())?;
    let keys = content
        .get("keys")
        .and_then(Value::as_str)
        .and_then(|k| Mac::from_base64(k).ok())
        .ok_or(())?;
    let mut ids: Vec<&str> = mac.keys().map(String::as_str).collect();
    ids.sort_unstable();
    established
        .verify_mac(&ids.join(","), &format!("{base}KEY_IDS"), &keys)
        .map_err(|_| ())?;
    let device_key_id = format!("ed25519:{}", peer.device.device_id);
    let master_b64 = peer.master_key.to_base64();
    let master_key_id = format!("ed25519:{master_b64}");
    // 端末の署名鍵とマスター鍵の両方の MAC がそろっていなければ受け付けない。
    if !mac.contains_key(&master_key_id) || !mac.contains_key(&device_key_id) {
        return Err(());
    }
    for (key_id, tag) in mac {
        let key = if *key_id == device_key_id {
            peer.device.ed25519.to_base64()
        } else if *key_id == master_key_id {
            master_b64.clone()
        } else {
            return Err(());
        };
        let tag = tag
            .as_str()
            .and_then(|t| Mac::from_base64(t).ok())
            .ok_or(())?;
        established
            .verify_mac(&key, &format!("{base}{key_id}"), &tag)
            .map_err(|_| ())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use vodozemac::Ed25519SecretKey;

    use super::*;
    use crate::verify_device_keys;

    const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
    const ALICE_DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const BOB: &str = "00000000-0000-0000-0000-00000000000b";
    const BOB_DEVICE: &str = "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    const TXN: &str = "6f1c2b3a-4d5e-4f60-8a7b-9c0d1e2f3a4b";

    fn party(user: &str, device: &str) -> (VerificationIdentity, Peer) {
        let account = DeviceAccount::new(user, device).unwrap();
        let master = Ed25519SecretKey::new().public_key();
        let keys = verify_device_keys(&account.device_keys(), user, device).unwrap();
        (
            VerificationIdentity {
                user_id: user.to_owned(),
                device_id: device.to_owned(),
                device_ed25519: keys.ed25519,
                master_key: master,
            },
            Peer {
                device: keys,
                master_key: master,
            },
        )
    }

    fn compare(established: EstablishedSas) -> Compare {
        let bytes = established.bytes("test");
        Compare {
            established,
            bytes,
            confirmed: true,
            their_mac: None,
            mac_verified: false,
            their_done: false,
        }
    }

    /// 端末の署名鍵の MAC を外し、鍵の一覧の MAC を正しく作り直しても受け付けない。
    #[test]
    fn mac_must_cover_the_device_key() {
        let (alice, alice_peer) = party(ALICE, ALICE_DEVICE);
        let (bob, bob_peer) = party(BOB, BOB_DEVICE);
        let (alice_sas, bob_sas) = (Sas::new(), Sas::new());
        let (alice_key, bob_key) = (alice_sas.public_key(), bob_sas.public_key());
        let alice_established = alice_sas.diffie_hellman(bob_key).unwrap();
        let bob_compare = compare(bob_sas.diffie_hellman(alice_key).unwrap());

        let full = own_mac(&bob, ALICE, Some(&alice_peer), TXN, &bob_compare).unwrap();
        assert!(verify_their_mac(&alice, &bob_peer, TXN, &alice_established, &full).is_ok());

        let base = mac_info_base(BOB, BOB_DEVICE, ALICE, ALICE_DEVICE, TXN);
        let master_key_id = format!("ed25519:{}", bob.master_key.to_base64());
        let mut master_only = full.clone();
        master_only["mac"]
            .as_object_mut()
            .unwrap()
            .remove(&format!("ed25519:{BOB_DEVICE}"));
        master_only["keys"] = json!(
            bob_compare
                .established
                .calculate_mac(&master_key_id, &format!("{base}KEY_IDS"))
                .to_base64()
        );
        assert!(
            verify_their_mac(&alice, &bob_peer, TXN, &alice_established, &master_only).is_err()
        );
    }
}
