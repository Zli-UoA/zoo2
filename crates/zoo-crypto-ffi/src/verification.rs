//! 他のユーザーの絵文字 SAS 検証（#368）と QR による検証（#685、iOS）。
//!
//! Web の `crates/zoo-crypto-wasm/src/verification.rs` と同じ形。検証のメッセージは JSON 文字列で受け渡し、
//! 送るメッセージ（[`VerificationOutgoing`]）は送り先の端末と `type`、`content`（JSON）にする。

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use zoo_crypto as core;

use crate::{
    AcceptedMasterKey, CrossSigningKeyPair, DeviceAccount, OlmDecrypted, OlmSessionStore, Result,
    TrustStore, VerifiedDeviceKeys, ZooCryptoError, lock, parse_json,
};

/// 送るメッセージ。`devices` のそれぞれに、`event_type` と `content` を to-device で送る。
#[derive(uniffi::Record)]
pub struct VerificationOutgoing {
    pub devices: Vec<Arc<VerifiedDeviceKeys>>,
    pub event_type: String,
    /// `content`（JSON）。
    pub content: String,
}

fn outgoing(out: Vec<core::VerificationOutgoing>) -> Vec<VerificationOutgoing> {
    out.into_iter()
        .map(|o| VerificationOutgoing {
            devices: o
                .devices
                .into_iter()
                .map(|inner| Arc::new(VerifiedDeviceKeys { inner }))
                .collect(),
            event_type: o.event_type.to_owned(),
            content: o.content.to_string(),
        })
        .collect()
}

fn snapshot(pages: &[String]) -> Result<core::KeySnapshot> {
    let mut snapshot = core::KeySnapshot::default();
    for page in pages {
        snapshot.extend(core::KeySnapshot::from_json(&parse_json(
            "snapshot_pages",
            page,
        )?)?);
    }
    Ok(snapshot)
}

/// 受信時の検証を通った検証のメッセージの JSON（`sender_user_id`、`sender_device`、`type`、`content`）を読む。
fn message(json: &str) -> Result<core::VerificationMessage> {
    let value = parse_json("message", json)?;
    let text = |name: &str| -> Result<String> {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| ZooCryptoError::InvalidPayload {
                reason: format!("message.{name} がありません"),
            })
    };
    let device =
        value
            .get("sender_device")
            .cloned()
            .ok_or_else(|| ZooCryptoError::InvalidPayload {
                reason: "message.sender_device がありません".into(),
            })?;
    let sender_device = serde_json::from_value::<DeviceKeysJson>(device)
        .map_err(|e| ZooCryptoError::InvalidPayload {
            reason: format!("sender_device が正しくありません: {e}"),
        })?
        .into_core()?;
    Ok(core::VerificationMessage {
        sender_user_id: text("sender_user_id")?,
        sender_device,
        event_type: text("type")?,
        content: value.get("content").cloned().unwrap_or(Value::Null),
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DeviceKeysJson {
    user_id: String,
    device_id: String,
    algorithms: Vec<String>,
    curve25519: String,
    ed25519: String,
}

impl DeviceKeysJson {
    fn into_core(self) -> Result<core::DeviceKeys> {
        Ok(core::DeviceKeys {
            user_id: self.user_id,
            device_id: self.device_id,
            algorithms: self.algorithms,
            curve25519: core::Curve25519PublicKey::from_base64(&self.curve25519).map_err(|e| {
                ZooCryptoError::InvalidPayload {
                    reason: format!("curve25519 が正しくありません: {e}"),
                }
            })?,
            ed25519: crate::ed25519_from_base64(&self.ed25519)?,
        })
    }
}

#[uniffi::export]
impl OlmDecrypted {
    /// 絵文字 SAS の検証のメッセージ（`m.key.verification.*`）を、受信時の検証の手順 1〜3 で確かめ、
    /// 検証のメッセージの JSON を返す（`SasVerification` に渡す）。`listed_device_keys` は、平文の
    /// `sender` と `sender_device` で探した `keys/query` の `device_keys`。なければ `null`（捨てる）。
    pub fn verify_verification(
        &self,
        account: &DeviceAccount,
        listed_device_keys: Option<String>,
    ) -> Result<String> {
        let listed = listed_device_keys
            .as_deref()
            .map(|json| parse_json("listed_device_keys", json))
            .transpose()?;
        let listed = match &listed {
            Some(value) => core::ListedDevice::Listed(value),
            None => core::ListedDevice::NotListed,
        };
        let account = lock(&account.inner);
        let m = self.inner.verify_verification(&account, listed)?;
        let device = &m.sender_device;
        Ok(json!({
            "sender_user_id": m.sender_user_id,
            "sender_device": {
                "user_id": device.user_id,
                "device_id": device.device_id,
                "algorithms": device.algorithms,
                "curve25519": device.curve25519.to_base64(),
                "ed25519": device.ed25519.to_base64(),
            },
            "type": m.event_type,
            "content": m.content,
        })
        .to_string())
    }
}

#[uniffi::export]
impl DeviceAccount {
    /// 検証のメッセージ（`event_type` は `m.key.verification.*`）を `recipient` に送る to-device の
    /// `content`（JSON）を作る。`session_id` は `sessions` にある、`recipient` との Olm セッション。
    pub fn encrypt_verification(
        &self,
        sessions: &OlmSessionStore,
        session_id: String,
        recipient: &VerifiedDeviceKeys,
        event_type: String,
        content: String,
    ) -> Result<String> {
        let content = parse_json("content", &content)?;
        let account = lock(&self.inner);
        let mut sessions = lock(&sessions.inner);
        let session = sessions
            .iter_mut()
            .find(|s| s.session_id() == session_id)
            .ok_or_else(|| ZooCryptoError::Olm {
                reason: format!("Olm セッション {session_id} がありません"),
            })?;
        Ok(account
            .encrypt_verification(session, &recipient.inner, &event_type, &content)?
            .to_string())
    }
}

/// [`SasVerification::request`] の結果。
#[derive(uniffi::Record)]
pub struct SasRequest {
    pub verification: Arc<SasVerification>,
    pub outgoing: Vec<VerificationOutgoing>,
}

/// 画面に出す、検証の今の状態。
#[derive(uniffi::Record)]
pub struct SasView {
    pub transaction_id: String,
    pub their_user_id: String,
    pub their_device_id: Option<String>,
    pub we_requested: bool,
    pub started_at_ms: u64,
    /// `requested`、`incoming`、`choose_method`、`qr_scanned`、`qr_confirm`、`exchanging`、`compare`、
    /// `confirmed`、`done`、`cancelled`。
    pub phase: String,
    /// 絵文字の番号（7 つ。比べる段階だけ）。
    pub emoji: Option<Vec<u8>>,
    pub cancel_code: Option<String>,
    pub cancelled_by_us: bool,
    /// 検証できた相手のマスター鍵（base64）。
    pub verified_master_key: Option<String>,
    /// `sas` か `qr`（方式が決まってから）。
    pub method: Option<String>,
    pub qr_code: Option<Vec<u8>>,
    pub can_scan_qr_code: bool,
    pub can_start_sas: bool,
}

/// 1 つの検証（`zoo_crypto::SasVerification`）。保存できない（メモリーにだけ置く）。
#[derive(uniffi::Object)]
pub struct SasVerification {
    inner: Mutex<core::SasVerification>,
}

fn identity(account: &DeviceAccount, trust: &TrustStore) -> Result<core::VerificationIdentity> {
    Ok(core::VerificationIdentity::new(
        &lock(&account.inner),
        &lock(&trust.inner),
    )?)
}

#[uniffi::export]
impl SasVerification {
    /// `their_user_id` に検証を求める。`snapshot_pages` は相手の `keys/query` の応答（JSON）。
    #[uniffi::constructor]
    pub fn request(
        account: &DeviceAccount,
        trust: &TrustStore,
        snapshot_pages: Vec<String>,
        their_user_id: String,
        now_ms: u64,
    ) -> Result<SasRequest> {
        let (inner, out) = core::SasVerification::request(
            identity(account, trust)?,
            &snapshot(&snapshot_pages)?,
            &their_user_id,
            now_ms,
        )?;
        Ok(SasRequest {
            verification: Arc::new(Self {
                inner: Mutex::new(inner),
            }),
            outgoing: outgoing(vec![out]),
        })
    }

    /// 届いた要求から検証を作る。古い要求、たどれない端末からの要求などは投げる（捨てる）。
    #[uniffi::constructor]
    pub fn from_request(
        account: &DeviceAccount,
        trust: &TrustStore,
        message_json: String,
        snapshot_pages: Vec<String>,
        now_ms: u64,
    ) -> Result<Arc<Self>> {
        let inner = core::SasVerification::from_request(
            identity(account, trust)?,
            &message(&message_json)?,
            &snapshot(&snapshot_pages)?,
            now_ms,
        )?;
        Ok(Arc::new(Self {
            inner: Mutex::new(inner),
        }))
    }

    pub fn view(&self) -> SasView {
        let v = lock(&self.inner);
        let phase = match v.phase() {
            core::SasPhase::Requested => "requested",
            core::SasPhase::Incoming => "incoming",
            core::SasPhase::ChooseMethod => "choose_method",
            core::SasPhase::QrScanned => "qr_scanned",
            core::SasPhase::QrConfirm => "qr_confirm",
            core::SasPhase::Exchanging => "exchanging",
            core::SasPhase::Compare => "compare",
            core::SasPhase::Confirmed => "confirmed",
            core::SasPhase::Done => "done",
            core::SasPhase::Cancelled => "cancelled",
        };
        SasView {
            transaction_id: v.transaction_id().to_owned(),
            their_user_id: v.their_user_id().to_owned(),
            their_device_id: v.their_device_id().map(str::to_owned),
            we_requested: v.we_requested(),
            started_at_ms: v.started_at_ms(),
            phase: phase.to_owned(),
            emoji: v.emoji_indices().map(|e| e.to_vec()),
            cancel_code: v.cancellation().map(|c| c.code.clone()),
            cancelled_by_us: v.cancellation().is_some_and(|c| c.by_us),
            verified_master_key: v.verified_master_key().map(|k| k.to_base64()),
            method: v.method().map(|m| {
                match m {
                    core::VerificationMethod::Sas => "sas",
                    core::VerificationMethod::QrCode => "qr",
                }
                .to_owned()
            }),
            qr_code: v.qr_code(),
            can_scan_qr_code: v.can_scan_qr_code(),
            can_start_sas: v.can_start_sas(),
        }
    }

    /// 要求を受け入れる。
    pub fn accept(&self) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).accept()?))
    }

    /// 絵文字が一致すると答えた。
    pub fn confirm(&self) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).confirm()?))
    }

    /// 絵文字で検証することを選んだ（SAS の `start` を送る）。
    pub fn start_sas(&self) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).start_sas()?))
    }

    /// 相手の QR を読み取った（`bytes` は QR のバイト列）。形が正しくない、別の検証の QR は投げる（読み直す）。
    pub fn scan_qr_code(&self, bytes: Vec<u8>) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).scan_qr_code(&bytes)?))
    }

    /// 相手がこの端末の QR を読み取ったことを、利用者が確かめた（`done` を送る）。
    pub fn confirm_qr_scanned(&self) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).confirm_qr_scanned()?))
    }

    /// 絵文字が一致しないと答えた。
    pub fn mismatch(&self) -> Vec<VerificationOutgoing> {
        outgoing(lock(&self.inner).mismatch())
    }

    /// 取りやめる（要求を断る）。
    pub fn cancel(&self) -> Vec<VerificationOutgoing> {
        outgoing(lock(&self.inner).cancel())
    }

    /// 10 分たっても終わっていなければ取りやめる。
    pub fn expire(&self, now_ms: u64) -> Vec<VerificationOutgoing> {
        outgoing(lock(&self.inner).expire(now_ms))
    }

    /// 届いたメッセージで進める。`snapshot_pages` は送信ユーザーの `keys/query` の応答（JSON）。
    pub fn receive(
        &self,
        message_json: String,
        snapshot_pages: Vec<String>,
    ) -> Result<Vec<VerificationOutgoing>> {
        Ok(outgoing(lock(&self.inner).receive(
            &message(&message_json)?,
            &snapshot(&snapshot_pages)?,
        )))
    }

    /// 検証できた相手のマスター鍵に署名する。`master_key` は署名する直前に `keys/query` で読んだ
    /// 相手のマスター鍵（JSON）。
    pub fn sign_master_key(
        &self,
        trust: &TrustStore,
        user_signing: &CrossSigningKeyPair,
        master_key: String,
    ) -> Result<AcceptedMasterKey> {
        let master_key = parse_json("master_key", &master_key)?;
        let accepted = lock(&self.inner).sign_master_key(
            &mut lock(&trust.inner),
            &user_signing.inner,
            &master_key,
        )?;
        Ok(AcceptedMasterKey {
            pin: accepted.pin.to_string(),
            signature: accepted.signature.into(),
        })
    }
}
