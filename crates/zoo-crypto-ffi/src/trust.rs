//! 配布先の判定と TOFU の記録（#350）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zoo_crypto as core;

use crate::{
    CrossSigningKeyPair, DeviceAccount, Result, SignatureUpload, VerifiedDeviceKeys,
    ZooCryptoError, bytes32, lock, parse_json,
};

/// 相手ごとの記録（[`core::TrustRecord`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrustRecord {
    pub master_key: String,
    pub confirmed_usk: Option<String>,
    pub pin_pending: bool,
}

/// TOFU で作ったピン（[`core::NewTofuPin`]）。`pin` は署名付きの `zoo.tofu_pin` の JSON。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewTofuPin {
    pub target_user_id: String,
    pub master_key: String,
    pub pin: String,
}

impl From<core::NewTofuPin> for NewTofuPin {
    fn from(p: core::NewTofuPin) -> Self {
        Self {
            target_user_id: p.target_user_id,
            master_key: p.master_key,
            pin: p.pin.to_string(),
        }
    }
}

impl TryFrom<NewTofuPin> for core::NewTofuPin {
    type Error = ZooCryptoError;

    fn try_from(p: NewTofuPin) -> Result<Self> {
        Ok(Self {
            pin: parse_json("pin", &p.pin)?,
            target_user_id: p.target_user_id,
            master_key: p.master_key,
        })
    }
}

/// マスター鍵を信頼した理由（[`core::TrustBasis`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TrustBasis {
    Own,
    UserSigningSignature,
    SharedPin,
    Record,
}

/// 再検証が必要な理由（[`core::ReverificationReason`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ReverificationReason {
    MasterKeyChanged,
    NotConfirmed,
    UserSigningKeyChanged,
}

/// ユーザーのマスター鍵の判定（[`core::UserState`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UserState {
    Trusted { basis: TrustBasis },
    NoCrossSigningKeys,
    InvalidCrossSigningKeys,
    NeedsReverification { reason: ReverificationReason },
    NeedsUnlock,
    PinPending,
    Blocked,
}

/// 端末に配らない理由（[`core::DeviceExclusion`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceExclusion {
    CurrentDevice,
    InvalidDeviceKeys,
    InvalidSelfSigningKey,
    Unsigned,
    BadSignature,
}

/// 利用者に出す警告（[`core::TrustWarning`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TrustWarning {
    OwnIdentityUnknown,
    OwnMasterKeyChanged,
    UntrustedUserSigningKey,
    UserSigningKeyRollback,
    UserSigningKeyChanged,
}

/// 1 人のユーザーの判定（[`core::UserTrust`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UserTrust {
    pub state: UserState,
    /// 端末 ID → 配らない理由。
    pub excluded_devices: HashMap<String, DeviceExclusion>,
}

/// 配布先の判定の結果（[`core::Distribution`]）。
#[derive(uniffi::Record)]
pub struct Distribution {
    /// Megolm セッション鍵を配る端末。
    pub devices: Vec<Arc<VerifiedDeviceKeys>>,
    /// ユーザー ID → 判定。
    pub users: HashMap<String, UserTrust>,
    /// 登録待ちのピン。
    pub new_pins: Vec<NewTofuPin>,
    pub warnings: Vec<TrustWarning>,
}

/// 利用者が新しいマスター鍵を受け入れたときに送るもの（[`core::AcceptedMasterKey`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AcceptedMasterKey {
    /// `POST /api/e2ee/tofu_pins` に `replace: true` で送るピン（JSON）。
    pub pin: String,
    pub signature: SignatureUpload,
}

/// 配布先の判定に使う記録（[`core::TrustStore`]）。
#[derive(uniffi::Object)]
pub struct TrustStore {
    inner: Mutex<core::TrustStore>,
}

#[uniffi::export]
impl TrustStore {
    /// 空の記録を作る。`user_id` は自分のユーザー ID。
    #[uniffi::constructor]
    pub fn new(user_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::TrustStore::new(&user_id)?),
        }))
    }

    /// [`to_json`](Self::to_json) で保存したものを戻す。
    #[uniffi::constructor]
    pub fn from_json(json: String, user_id: String) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::TrustStore::from_json(&json, &user_id)?),
        }))
    }

    /// [`pickle`](Self::pickle) で保存したものを戻す。`account` は今の端末。封筒を開けないときは
    /// `InvalidPickle`、中の JSON が正しくない、または `account` のユーザーの記録でないときは
    /// `InvalidTrustStore`。
    #[uniffi::constructor]
    pub fn from_pickle(
        pickle: String,
        account: &DeviceAccount,
        storage_key: Vec<u8>,
    ) -> Result<Arc<Self>> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        Ok(Arc::new(Self {
            inner: Mutex::new(core::TrustStore::from_pickle(
                &pickle,
                &account,
                &storage_key,
            )?),
        }))
    }

    /// 保存形式（JSON）。端末アカウントと同じ保存鍵で暗号化して置く。
    pub fn to_json(&self) -> String {
        lock(&self.inner).to_json()
    }

    /// [`to_json`](Self::to_json) の JSON を、32 バイトの保存鍵で暗号化した、暗号ストアに置く
    /// 文字列にする（#359）。`account` は今の端末。保存のたびに出力は変わる。
    ///
    /// ロックは端末アカウント、このオブジェクトの順に取る。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let account = lock(&account.inner);
        Ok(lock(&self.inner).pickle(&account, &storage_key))
    }

    pub fn user_id(&self) -> String {
        lock(&self.inner).user_id().to_owned()
    }

    pub fn own_master_key(&self) -> Option<String> {
        lock(&self.inner).own_master_key().map(str::to_owned)
    }

    pub fn user_signing_key_history(&self) -> Vec<String> {
        lock(&self.inner).user_signing_key_history().to_vec()
    }

    pub fn record(&self, target_user_id: String) -> Option<TrustRecord> {
        lock(&self.inner)
            .record(&target_user_id)
            .map(|r| TrustRecord {
                master_key: r.master_key.clone(),
                confirmed_usk: r.confirmed_usk.clone(),
                pin_pending: r.pin_pending,
            })
    }

    /// 初回設定で作った、または SSSS から取り出した自分のマスター鍵を信頼する。
    pub fn trust_own_master_key(&self, master: &CrossSigningKeyPair) -> Result<()> {
        Ok(lock(&self.inner).trust_own_master_key(&master.inner)?)
    }

    /// 自分のユーザー署名鍵を履歴に足す。
    pub fn record_own_user_signing_key(&self, user_signing: &CrossSigningKeyPair) -> Result<()> {
        Ok(lock(&self.inner).record_own_user_signing_key(&user_signing.inner)?)
    }

    /// 利用者が受け入れた相手の新しいマスター鍵（JSON）を記録し、送るピンと署名を作る。
    pub fn accept_master_key(
        &self,
        user_signing: &CrossSigningKeyPair,
        target_user_id: String,
        master_key: String,
    ) -> Result<AcceptedMasterKey> {
        let master_key = parse_json("master_key", &master_key)?;
        let accepted = lock(&self.inner).accept_master_key(
            &user_signing.inner,
            &target_user_id,
            &master_key,
        )?;
        Ok(AcceptedMasterKey {
            pin: accepted.pin.to_string(),
            signature: accepted.signature.into(),
        })
    }

    /// [`accept_master_key`](Self::accept_master_key) のピンと署名をサーバーに保存できた。
    /// 記録の登録待ちを外す（#368）。
    pub fn confirm_accepted_master_key(&self, target_user_id: String, master_key: String) {
        lock(&self.inner).confirm_accepted_master_key(&target_user_id, &master_key);
    }

    /// ピンの登録が成功した。
    pub fn confirm_tofu_pin(&self, pin: NewTofuPin) -> Result<()> {
        lock(&self.inner).confirm_tofu_pin(&pin.try_into()?);
        Ok(())
    }

    /// ピンの登録が 409 `E2EE_PIN_CONFLICT` だった。
    pub fn discard_tofu_pin(&self, pin: NewTofuPin) -> Result<()> {
        lock(&self.inner).discard_tofu_pin(&pin.try_into()?);
        Ok(())
    }

    /// 配布先を判定する。`snapshot_pages` は同じ `membership_epoch` のスナップショット（JSON）のページ。
    /// 記録が変わるので、呼んだあとは保存し直す。
    pub fn evaluate(
        &self,
        snapshot_pages: Vec<String>,
        own_device_id: String,
        user_signing: Option<Arc<CrossSigningKeyPair>>,
    ) -> Result<Distribution> {
        let mut snapshot = core::KeySnapshot::default();
        for page in &snapshot_pages {
            snapshot.extend(core::KeySnapshot::from_json(&parse_json(
                "snapshot_pages",
                page,
            )?)?);
        }
        let distribution = lock(&self.inner).evaluate(
            &snapshot,
            &own_device_id,
            user_signing.as_ref().map(|k| &k.inner),
        );
        Ok(distribution.into())
    }
}

impl From<core::Distribution> for Distribution {
    fn from(d: core::Distribution) -> Self {
        Self {
            devices: d
                .devices
                .into_iter()
                .map(|inner| Arc::new(VerifiedDeviceKeys { inner }))
                .collect(),
            users: d
                .users
                .into_iter()
                .map(|(user_id, trust)| {
                    let trust = UserTrust {
                        state: trust.state.into(),
                        excluded_devices: trust
                            .excluded_devices
                            .into_iter()
                            .map(|(device_id, e)| (device_id, e.into()))
                            .collect(),
                    };
                    (user_id, trust)
                })
                .collect(),
            new_pins: d.new_pins.into_iter().map(Into::into).collect(),
            warnings: d.warnings.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<core::UserState> for UserState {
    fn from(state: core::UserState) -> Self {
        match state {
            core::UserState::Trusted(basis) => Self::Trusted {
                basis: match basis {
                    core::TrustBasis::Own => TrustBasis::Own,
                    core::TrustBasis::UserSigningSignature => TrustBasis::UserSigningSignature,
                    core::TrustBasis::SharedPin => TrustBasis::SharedPin,
                    core::TrustBasis::Record => TrustBasis::Record,
                },
            },
            core::UserState::NoCrossSigningKeys => Self::NoCrossSigningKeys,
            core::UserState::InvalidCrossSigningKeys => Self::InvalidCrossSigningKeys,
            core::UserState::NeedsReverification(reason) => Self::NeedsReverification {
                reason: match reason {
                    core::ReverificationReason::MasterKeyChanged => {
                        ReverificationReason::MasterKeyChanged
                    }
                    core::ReverificationReason::NotConfirmed => ReverificationReason::NotConfirmed,
                    core::ReverificationReason::UserSigningKeyChanged => {
                        ReverificationReason::UserSigningKeyChanged
                    }
                },
            },
            core::UserState::NeedsUnlock => Self::NeedsUnlock,
            core::UserState::PinPending => Self::PinPending,
            core::UserState::Blocked => Self::Blocked,
        }
    }
}

impl From<core::DeviceExclusion> for DeviceExclusion {
    fn from(e: core::DeviceExclusion) -> Self {
        match e {
            core::DeviceExclusion::CurrentDevice => Self::CurrentDevice,
            core::DeviceExclusion::InvalidDeviceKeys => Self::InvalidDeviceKeys,
            core::DeviceExclusion::InvalidSelfSigningKey => Self::InvalidSelfSigningKey,
            core::DeviceExclusion::Unsigned => Self::Unsigned,
            core::DeviceExclusion::BadSignature => Self::BadSignature,
        }
    }
}

impl From<core::TrustWarning> for TrustWarning {
    fn from(w: core::TrustWarning) -> Self {
        match w {
            core::TrustWarning::OwnIdentityUnknown => Self::OwnIdentityUnknown,
            core::TrustWarning::OwnMasterKeyChanged => Self::OwnMasterKeyChanged,
            core::TrustWarning::UntrustedUserSigningKey => Self::UntrustedUserSigningKey,
            core::TrustWarning::UserSigningKeyRollback => Self::UserSigningKeyRollback,
            core::TrustWarning::UserSigningKeyChanged => Self::UserSigningKeyChanged,
        }
    }
}
