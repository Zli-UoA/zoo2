//! 他のユーザーの絵文字 SAS 検証（#368）と、QR コードによる検証。Web の
//! `crates/zoo-crypto-wasm/src/verification.rs` と同じ API を Android から使う。
//!
//! 検証のメッセージ（[`VerificationMessage`]）と送るメッセージ（[`VerificationOutgoing`]）は
//! Record にする。`content` はコアと同じ形の JSON 文字列。宛先と送信端末は検証済みの端末鍵
//! （[`VerifiedDeviceKeys`]）のまま渡すので、Web の `DeviceKeys.fromVerificationTarget` は要らない。
//!
//! vodozemac の SAS の状態は保存できないので、[`SasVerification`] はメモリーにだけ置く。

use std::sync::{Arc, Mutex};

use zoo_crypto as core;

use crate::{
    AcceptedMasterKey, CrossSigningKeyPair, DeviceAccount, OlmDecrypted, OlmSessionStore, Result,
    TrustStore, VerifiedDeviceKeys, ZooCryptoError, lock, parse_json,
};

fn snapshot(keys_query: &str) -> Result<core::KeySnapshot> {
    Ok(core::KeySnapshot::from_json(&parse_json(
        "keys_query",
        keys_query,
    )?)?)
}

fn device(keys: core::DeviceKeys) -> Arc<VerifiedDeviceKeys> {
    Arc::new(VerifiedDeviceKeys { inner: keys })
}

/// 受信時の検証を通った検証のメッセージ（[`core::VerificationMessage`]）。
/// [`OlmDecrypted::verify_verification`] で作り、[`SasVerification::from_request`] と
/// [`SasVerification::receive`] に渡す。
#[derive(uniffi::Record)]
pub struct VerificationMessage {
    /// 送信ユーザー。
    pub sender_user_id: String,
    /// 送信端末の端末鍵（Olm セッションの相手と照合済み）。
    pub sender_device: Arc<VerifiedDeviceKeys>,
    /// `m.key.verification.*`。
    pub event_type: String,
    /// `content`（JSON オブジェクト）。`transaction_id` はここにある。
    pub content: String,
}

impl VerificationMessage {
    fn to_core(&self) -> Result<core::VerificationMessage> {
        Ok(core::VerificationMessage {
            sender_user_id: self.sender_user_id.clone(),
            sender_device: self.sender_device.inner.clone(),
            event_type: self.event_type.clone(),
            content: parse_json("content", &self.content)?,
        })
    }
}

/// 送るメッセージ（[`core::VerificationOutgoing`]）。`devices` のそれぞれに
/// [`DeviceAccount::encrypt_verification`] で暗号化して送る。
#[derive(uniffi::Record)]
pub struct VerificationOutgoing {
    /// 宛先の端末。
    pub devices: Vec<Arc<VerifiedDeviceKeys>>,
    /// `m.key.verification.*`。
    pub event_type: String,
    /// `content`（JSON オブジェクト）。
    pub content: String,
}

fn outgoing(out: Vec<core::VerificationOutgoing>) -> Vec<VerificationOutgoing> {
    out.into_iter()
        .map(|o| VerificationOutgoing {
            devices: o.devices.into_iter().map(device).collect(),
            event_type: o.event_type.to_owned(),
            content: o.content.to_string(),
        })
        .collect()
}

/// 利用者に見せる検証の段階（[`core::SasPhase`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SasPhase {
    Requested,
    Incoming,
    ChooseMethod,
    QrScanned,
    QrConfirm,
    Exchanging,
    Compare,
    Confirmed,
    Done,
    Cancelled,
}

impl From<core::SasPhase> for SasPhase {
    fn from(phase: core::SasPhase) -> Self {
        match phase {
            core::SasPhase::Requested => Self::Requested,
            core::SasPhase::Incoming => Self::Incoming,
            core::SasPhase::ChooseMethod => Self::ChooseMethod,
            core::SasPhase::QrScanned => Self::QrScanned,
            core::SasPhase::QrConfirm => Self::QrConfirm,
            core::SasPhase::Exchanging => Self::Exchanging,
            core::SasPhase::Compare => Self::Compare,
            core::SasPhase::Confirmed => Self::Confirmed,
            core::SasPhase::Done => Self::Done,
            core::SasPhase::Cancelled => Self::Cancelled,
        }
    }
}

/// 検証に使った方式（[`core::VerificationMethod`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VerificationMethod {
    /// 絵文字 SAS（`m.sas.v1`）。
    Sas,
    /// QR コード（`m.reciprocate.v1`）。
    QrCode,
}

/// 取りやめた理由（[`core::SasCancellation`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SasCancellation {
    /// `m.key.verification.cancel` の `code`（`m.user`、`m.timeout`、`m.mismatched_sas` など）。
    pub code: String,
    /// この端末が取りやめたか（`false` なら相手から届いた）。
    pub by_us: bool,
}

/// 検証の今の状態（Web の `SasView`）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SasView {
    pub transaction_id: String,
    pub their_user_id: String,
    pub their_device_id: Option<String>,
    pub we_requested: bool,
    /// 始めた時刻（Unix 時間のミリ秒）。
    pub started_at: u64,
    pub phase: SasPhase,
    /// 絵文字の番号（0〜63 が 7 つ。`Compare` 以降）。
    pub emoji: Option<Vec<u8>>,
    pub cancel: Option<SasCancellation>,
    /// 検証できた相手のマスター鍵（base64）。
    pub verified_master_key: Option<String>,
    pub method: Option<VerificationMethod>,
    /// この端末が出す QR のバイト列（出せる段階だけ）。
    pub qr_code: Option<Vec<u8>>,
    pub can_scan_qr_code: bool,
    pub can_start_sas: bool,
}

/// [`request_verification`] の結果。`outgoing` は相手の端末に送る要求。
#[derive(uniffi::Record)]
pub struct SasRequest {
    pub verification: Arc<SasVerification>,
    pub outgoing: Vec<VerificationOutgoing>,
}

/// [`verification_devices`] の結果。
#[derive(uniffi::Record)]
pub struct VerificationDevices {
    /// サーバーの今のマスター鍵（base64）。信頼するかどうかは問わない。
    pub master_key: String,
    /// マスター鍵から自己署名鍵を経て署名をたどれる端末。
    pub devices: Vec<Arc<VerifiedDeviceKeys>>,
}

/// `user_id` の、サーバーの今のマスター鍵から署名をたどれる端末と、そのマスター鍵
/// （[`core::verification_devices`]）。`keys_query` は `keys/query` の応答（JSON）。
#[uniffi::export]
pub fn verification_devices(keys_query: String, user_id: String) -> Result<VerificationDevices> {
    let (master, devices) = core::verification_devices(&snapshot(&keys_query)?, &user_id)?;
    Ok(VerificationDevices {
        master_key: master.to_base64(),
        devices: devices.into_iter().map(device).collect(),
    })
}

fn identity(account: &DeviceAccount, trust: &TrustStore) -> Result<core::VerificationIdentity> {
    let account = lock(&account.inner);
    let trust = lock(&trust.inner);
    Ok(core::VerificationIdentity::new(&account, &trust)?)
}

/// `their_user_id` に検証を求める（Web の `SasVerification.request`。UniFFI のコンストラクターは
/// 自分しか返せないので関数にする）。`keys_query` は相手の `keys/query` の応答（JSON）、`now_ms` は
/// 今の時刻（Unix 時間のミリ秒）。解錠の前（自分のマスター鍵を信頼していない）は `Verification`。
///
/// ロックは端末アカウント、`trust` の順に取る。
#[uniffi::export]
pub fn request_verification(
    account: &DeviceAccount,
    trust: &TrustStore,
    keys_query: String,
    their_user_id: String,
    now_ms: u64,
) -> Result<SasRequest> {
    let (inner, out) = core::SasVerification::request(
        identity(account, trust)?,
        &snapshot(&keys_query)?,
        &their_user_id,
        now_ms,
    )?;
    Ok(SasRequest {
        verification: Arc::new(SasVerification {
            inner: Mutex::new(inner),
        }),
        outgoing: outgoing(vec![out]),
    })
}

/// 1 つの SAS の検証（[`core::SasVerification`]）。保存はできない（メモリーにだけ置く）。
#[derive(uniffi::Object)]
pub struct SasVerification {
    inner: Mutex<core::SasVerification>,
}

#[uniffi::export]
impl SasVerification {
    /// 届いた要求（`m.key.verification.request`）から検証を作る。`keys_query` は送信ユーザーの
    /// `keys/query` の応答（JSON）。古い要求、たどれない端末からの要求などはエラー（捨てる。
    /// 取りやめも送らない）。
    #[uniffi::constructor]
    pub fn from_request(
        account: &DeviceAccount,
        trust: &TrustStore,
        message: VerificationMessage,
        keys_query: String,
        now_ms: u64,
    ) -> Result<Arc<Self>> {
        let inner = core::SasVerification::from_request(
            identity(account, trust)?,
            &message.to_core()?,
            &snapshot(&keys_query)?,
            now_ms,
        )?;
        Ok(Arc::new(Self {
            inner: Mutex::new(inner),
        }))
    }

    pub fn transaction_id(&self) -> String {
        lock(&self.inner).transaction_id().to_owned()
    }

    /// 今の状態（画面に出すもの）。
    pub fn view(&self) -> SasView {
        let v = lock(&self.inner);
        SasView {
            transaction_id: v.transaction_id().to_owned(),
            their_user_id: v.their_user_id().to_owned(),
            their_device_id: v.their_device_id().map(str::to_owned),
            we_requested: v.we_requested(),
            started_at: v.started_at_ms(),
            phase: v.phase().into(),
            emoji: v.emoji_indices().map(|e| e.to_vec()),
            cancel: v.cancellation().map(|c| SasCancellation {
                code: c.code.clone(),
                by_us: c.by_us,
            }),
            verified_master_key: v.verified_master_key().map(|k| k.to_base64()),
            method: v.method().map(|m| match m {
                core::VerificationMethod::Sas => VerificationMethod::Sas,
                core::VerificationMethod::QrCode => VerificationMethod::QrCode,
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

    /// 相手の QR を読み取った（`bytes` は QR のバイト列）。形が正しくない、別の検証の QR は
    /// `InvalidPayload` か `Verification`（取りやめない。読み直す）。鍵が違えば取りやめる
    /// （`view().phase` が `Cancelled`）。
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

    /// 届いたメッセージで進める。`keys_query` は送信ユーザーの `keys/query` の応答（JSON）。
    pub fn receive(
        &self,
        message: VerificationMessage,
        keys_query: String,
    ) -> Result<Vec<VerificationOutgoing>> {
        let message = message.to_core()?;
        let snapshot = snapshot(&keys_query)?;
        Ok(outgoing(lock(&self.inner).receive(&message, &snapshot)))
    }

    /// 検証できた相手のマスター鍵に、自分のユーザー署名鍵で署名する（`replace: true` のピンと署名。
    /// 記録は登録待ちになるので、呼んだあとは `trust` を保存し直し、サーバーに保存できたら
    /// `TrustStore.confirm_accepted_master_key`）。`master_key` は署名する直前に `keys/query` で
    /// 読んだ相手のマスター鍵（JSON）。
    ///
    /// ロックはこのオブジェクト、`trust` の順に取る。
    pub fn sign_master_key(
        &self,
        trust: &TrustStore,
        user_signing: &CrossSigningKeyPair,
        master_key: String,
    ) -> Result<AcceptedMasterKey> {
        let master_key = parse_json("master_key", &master_key)?;
        let verification = lock(&self.inner);
        let accepted = verification.sign_master_key(
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

#[uniffi::export]
impl OlmDecrypted {
    /// 検証のメッセージ（`type` が `m.key.verification.*`）を、受信時の検証の手順 1〜3 で確かめる。
    /// `type` は [`event_type`](Self::event_type) で見分ける。
    ///
    /// `listed_device_keys` は、平文の `sender` と `sender_device` で探した `keys/query` の応答の
    /// `device_keys`（JSON）。なければ `null`（`Rejected` で捨てる）。
    pub fn verify_verification(
        &self,
        account: &DeviceAccount,
        listed_device_keys: Option<String>,
    ) -> Result<VerificationMessage> {
        let listed = listed_device_keys
            .as_deref()
            .map(|json| parse_json("listed_device_keys", json))
            .transpose()?;
        let listed = match &listed {
            Some(value) => core::ListedDevice::Listed(value),
            None => core::ListedDevice::NotListed,
        };
        let message = self
            .inner
            .verify_verification(&lock(&account.inner), listed)?;
        Ok(VerificationMessage {
            sender_user_id: message.sender_user_id,
            sender_device: device(message.sender_device),
            event_type: message.event_type,
            content: message.content.to_string(),
        })
    }
}

#[uniffi::export]
impl DeviceAccount {
    /// 検証のメッセージ（`event_type` は `m.key.verification.*`、`content` は JSON）を `recipient`
    /// に送る to-device の `content`（JSON）を作る。`session_id` は `sessions` にある、`recipient` の
    /// 識別鍵とのセッション（なければ `keys/claim` の鍵で
    /// [`create_outbound_olm_session`](Self::create_outbound_olm_session) してから呼ぶ）。
    /// 暗号化でセッションが進むので、呼んだあとは `sessions` を保存し直す。
    ///
    /// ロックは端末アカウント、`sessions` の順に取る。
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
