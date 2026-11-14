//! 鍵の配布先の判定（docs/e2ee.md の「鍵の配布先の判定」の規則 1〜6）と、そのための記録。
//!
//! Megolm セッション鍵は、信頼したマスター鍵からクロス署名を検証できた端末にだけ配る。
//! [`TrustStore`] は暗号ストアに置く記録で、次を持つ。
//!
//! - 自分のマスター鍵（初回設定で作ったとき、または SSSS から取り出した秘密鍵で確かめたときに信頼する）
//! - 自分のユーザー署名鍵の公開鍵の履歴（一度置き換わった鍵への巻き戻しの検出に使う）
//! - 相手ごとの記録（TOFU で信頼したマスター鍵と、それを確認したユーザー署名鍵 `confirmed_usk`、
//!   記録から置き換えた古いマスター鍵の履歴 `replaced_master_keys`）

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vodozemac::Ed25519PublicKey;

use crate::cross_signing::{SignatureCheck, check_signature};
use crate::device::{DeviceAccount, validate_uuid};
use crate::store::{self, StoreKind};
use crate::{
    CrossSigningKeyPair, CryptoError, DeviceKeys, KeySignature, KeyUsage, SenderTrust,
    SignatureUpload, TOFU_PIN_TYPE, cross_signing_key_id, verify_cross_signing_key,
    verify_device_keys, verify_json_signature,
};
use zeroize::Zeroizing;

/// 保存形式の版。形式を変えたら上げる。
const TRUST_STORE_VERSION: u32 = 1;

/// 相手ごとの記録（暗号ストアの記録）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustRecord {
    /// 信頼した相手のマスター鍵の公開鍵（パディングなしの base64）。
    pub master_key: String,
    /// このマスター鍵を、どのユーザー署名鍵の公開鍵のもとで確認したか。
    /// `None` は、共有ピンを採用したがマスター鍵が今のものと違った記録で、再検証か SAS でだけ設定する。
    pub confirmed_usk: Option<String>,
    /// TOFU（規則 4）で作ったピンを、まだサーバーに置けていない。登録待ちの相手には配らず、
    /// 判定のたびにピンを [`Distribution::new_pins`] に返し直す。登録できたら
    /// [`TrustStore::confirm_tofu_pin`] で外す（スナップショットに同じピンが見えたときも外す）。
    #[serde(default)]
    pub pin_pending: bool,
    /// この相手について、記録から置き換えた古いマスター鍵（古い順。今の `master_key` は含まない）。
    ///
    /// 古いマスター鍵と、自分が昔それに付けた署名やピンを見せられても、記録を巻き戻さないために使う
    /// （docs/e2ee.md の脅威モデルの「相手のマスター鍵の巻き戻し」）。履歴にある鍵は、規則 3 の署名や
    /// 共有ピンがあっても信頼せず、[`ReverificationReason::MasterKeyChanged`] にする。履歴から外せるのは、
    /// 利用者が明示的に受け入れたとき（[`TrustStore::accept_master_key`]）だけ。
    ///
    /// 保存形式に後から足した項目なので、ない記録（以前の版で保存したもの）は空として読む。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced_master_keys: Vec<String>,
}

/// 配布先の判定に使う、暗号ストアの記録。
///
/// [`to_json`](Self::to_json) で文字列にし、呼び出し側が暗号ストア（#359）に暗号化して保存する。
/// [`evaluate`](Self::evaluate) などで変わるので、呼んだあとは保存し直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustStore {
    user_id: String,
    own_master_key: Option<String>,
    usk_history: Vec<String>,
    records: BTreeMap<String, TrustRecord>,
}

#[derive(Serialize, Deserialize)]
struct TrustStoreJson {
    version: u32,
    user_id: String,
    own_master_key: Option<String>,
    user_signing_key_history: Vec<String>,
    records: BTreeMap<String, TrustRecord>,
}

/// 配布先のスナップショット（`GET /api/e2ee/channels/{channel_id}/recipients`）か
/// `keys/query` の応答のうち、配布先の判定に使う部分。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KeySnapshot {
    /// 判定するユーザー（スナップショットの `user_ids`）。自分も含める。
    #[serde(default)]
    pub user_ids: Vec<String>,
    /// `<user_id>` → `<device_id>` → 署名付きの端末鍵。
    #[serde(default)]
    pub device_keys: BTreeMap<String, BTreeMap<String, Value>>,
    /// `<user_id>` → マスター鍵。
    #[serde(default)]
    pub master_keys: BTreeMap<String, Value>,
    /// `<user_id>` → 自己署名鍵。
    #[serde(default)]
    pub self_signing_keys: BTreeMap<String, Value>,
    /// 自分のユーザー署名鍵（本人にだけ返る）。
    #[serde(default)]
    pub user_signing_keys: BTreeMap<String, Value>,
    /// `e2ee_key_signatures` の行。
    #[serde(default)]
    pub signatures: Vec<KeySignature>,
    /// `<相手の user_id>` → 自分のピン。
    #[serde(default)]
    pub tofu_pins: BTreeMap<String, Value>,
}

impl KeySnapshot {
    /// サーバーの JSON から読む。
    pub fn from_json(value: &Value) -> Result<Self, CryptoError> {
        Self::deserialize(value)
            .map_err(|e| CryptoError::InvalidPayload(format!("スナップショットを読めません: {e}")))
    }

    /// 同じ `membership_epoch` の次のページを足す。
    pub fn extend(&mut self, page: KeySnapshot) {
        self.user_ids.extend(page.user_ids);
        self.device_keys.extend(page.device_keys);
        self.master_keys.extend(page.master_keys);
        self.self_signing_keys.extend(page.self_signing_keys);
        self.user_signing_keys.extend(page.user_signing_keys);
        self.signatures.extend(page.signatures);
        self.tofu_pins.extend(page.tofu_pins);
    }
}

/// 配布先の判定の結果。
#[derive(Debug, Clone, Default)]
pub struct Distribution {
    /// Megolm セッション鍵を配る端末（ユーザー ID と端末 ID の順）。
    pub devices: Vec<DeviceKeys>,
    /// ユーザーごとの判定。送信画面の表示（#361）に使う。
    pub users: BTreeMap<String, UserTrust>,
    /// TOFU（規則 4）で作った、登録待ちのピン。`POST /api/e2ee/tofu_pins`（`replace: false`）で
    /// サーバーに置く。成功したら [`TrustStore::confirm_tofu_pin`]、409 `E2EE_PIN_CONFLICT` なら
    /// [`TrustStore::discard_tofu_pin`] を呼び、判定し直す（409 では返ったピンをスナップショットに入れる）。
    /// 登録待ちの相手の端末は、登録できるまで [`devices`](Self::devices) に入らない。登録待ちは記録に
    /// 残るので、ピンを置く前に終了しても、次の判定で同じピンをここに返し直す。
    pub new_pins: Vec<NewTofuPin>,
    /// 利用者に出す警告。
    pub warnings: Vec<TrustWarning>,
    /// 記録と違う新しいマスター鍵に自分のユーザー署名鍵の署名があったので（規則 3）、記録を置き換えた
    /// 相手の user_id。自分の別の端末で受け入れた（#362）か SAS で検証した（#368）鍵なので配るが、
    /// 黙って置き換えないよう、「この相手の鍵が変わり、別の端末で確認済み」と利用者に知らせる。
    /// 置き換えた古い鍵は [`TrustRecord::replaced_master_keys`] に移り、以後その鍵には戻らない。
    pub signed_master_key_changes: Vec<String>,
}

/// TOFU で作ったピン。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTofuPin {
    /// 相手の user_id。
    pub target_user_id: String,
    /// 相手のマスター鍵の公開鍵。
    pub master_key: String,
    /// 署名付きの `zoo.tofu_pin`。
    pub pin: Value,
}

/// 1 人のユーザーの判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTrust {
    /// マスター鍵を信頼できたか。
    pub state: UserState,
    /// マスター鍵を信頼できたユーザーのうち、配らない端末と理由。
    pub excluded_devices: BTreeMap<String, DeviceExclusion>,
}

/// ユーザーのマスター鍵の判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserState {
    /// マスター鍵を信頼した。端末ごとの判定は規則 6（自分は規則 1）。
    Trusted(TrustBasis),
    /// クロス署名鍵がない（規則 2）。「暗号化を設定していない」と表示する。
    NoCrossSigningKeys,
    /// マスター鍵の JSON が壊れている。配らない。
    InvalidCrossSigningKeys,
    /// 再検証が必要（規則 5）。再検証（#362）か SAS（#368）まで、どの端末にも配らない。
    NeedsReverification(ReverificationReason),
    /// 初めて見た相手だが、TOFU のピンを作るユーザー署名鍵の秘密鍵がない（解錠前）。配らない。
    NeedsUnlock,
    /// 初めて見た相手（規則 4）で、TOFU のピンをまだサーバーに置けていない。
    /// [`Distribution::new_pins`] のピンを置き、[`TrustStore::confirm_tofu_pin`] を呼んでから判定し直す。
    PinPending,
    /// 自分のマスター鍵かユーザー署名鍵を確かめられないので配らない。理由は [`Distribution::warnings`]。
    Blocked,
}

/// マスター鍵を信頼した理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustBasis {
    /// 自分のマスター鍵（規則 1）。
    Own,
    /// 自分のユーザー署名鍵による署名（規則 3）。
    UserSigningSignature,
    /// 自分の別の端末が作ったピンを採用した。
    SharedPin,
    /// 暗号ストアの記録と一致した。
    Record,
}

/// 再検証が必要な理由（規則 5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReverificationReason {
    /// 記録かピンのマスター鍵と、今のマスター鍵が違う。
    MasterKeyChanged,
    /// 記録に `confirmed_usk` がない。
    NotConfirmed,
    /// ユーザー署名鍵が変わり、署名し直されたピンを確かめられない。
    UserSigningKeyChanged,
}

/// 端末に配らない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceExclusion {
    /// 今の端末（自分には配らない）。
    CurrentDevice,
    /// 端末鍵の JSON が壊れている、または自己署名が正しくない。
    InvalidDeviceKeys,
    /// 自己署名鍵がない、壊れている、またはマスター鍵で署名されていない。
    InvalidSelfSigningKey,
    /// 自己署名鍵で署名されていない。
    Unsigned,
    /// 自己署名鍵の署名が壊れている。
    BadSignature,
}

/// 利用者に出す警告。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustWarning {
    /// 自分のマスター鍵をまだ信頼していない（初回設定か解錠の前）。どの端末にも配らない。
    OwnIdentityUnknown,
    /// サーバーの自分のマスター鍵が、信頼したものと違う。SSSS で解錠し直すまで配らない。
    OwnMasterKeyChanged,
    /// 自分のユーザー署名鍵がない、または自分のマスター鍵で署名されていない。他のユーザーには配らない。
    UntrustedUserSigningKey,
    /// 自分のユーザー署名鍵が、一度置き換わった鍵に戻った。他のユーザーには配らない。
    UserSigningKeyRollback,
    /// 自分のユーザー署名鍵が変わった（履歴に足した）。署名し直されたピンがない相手は再検証になる。
    UserSigningKeyChanged,
}

/// 利用者が新しいマスター鍵を受け入れたとき（#362）と、SAS で検証したとき（#368）にサーバーに送るもの。
///
/// ピンと署名を送れたら、[`TrustStore::confirm_accepted_master_key`] で記録の登録待ちを外す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedMasterKey {
    /// 相手の user_id。
    pub target_user_id: String,
    /// 相手のマスター鍵の公開鍵。
    pub master_key: String,
    /// `POST /api/e2ee/tofu_pins` に `replace: true` で送るピン。
    pub pin: Value,
    /// `POST /api/e2ee/cross_signing/signatures` に送る、マスター鍵への署名。
    pub signature: SignatureUpload,
}

impl TrustStore {
    /// 空の記録を作る。`user_id` は自分のユーザー ID。
    pub fn new(user_id: &str) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id, false)?;
        Ok(Self {
            user_id: user_id.to_owned(),
            own_master_key: None,
            usk_history: Vec::new(),
            records: BTreeMap::new(),
        })
    }

    /// 保存する文字列（JSON）にする。
    pub fn to_json(&self) -> String {
        let json = TrustStoreJson {
            version: TRUST_STORE_VERSION,
            user_id: self.user_id.clone(),
            own_master_key: self.own_master_key.clone(),
            user_signing_key_history: self.usk_history.clone(),
            records: self.records.clone(),
        };
        serde_json::to_string(&json).expect("文字列だけの構造体は必ず JSON にできる")
    }

    /// [`to_json`](Self::to_json) の JSON を、暗号ストアに置く文字列にする（#359。形式は
    /// `store` モジュールの封筒）。`account` は今の端末。呼ぶたびに出力は変わる。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8; 32]) -> String {
        let json = Zeroizing::new(self.to_json());
        store::seal(storage_key, StoreKind::TrustStore, account, json.as_bytes())
    }

    /// [`pickle`](Self::pickle) の文字列から戻す。封筒を開けないときは [`CryptoError::InvalidPickle`]、
    /// 中の JSON が正しくない、または `account` のユーザーの記録でないときは
    /// [`CryptoError::InvalidTrustStore`]。
    pub fn from_pickle(
        pickle: &str,
        account: &DeviceAccount,
        storage_key: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        let plaintext = store::open(pickle, storage_key, StoreKind::TrustStore, account)?;
        let json = std::str::from_utf8(&plaintext)
            .map_err(|_| CryptoError::InvalidTrustStore("UTF-8 ではありません".into()))?;
        Self::from_json(json, account.user_id())
    }

    /// [`to_json`](Self::to_json) の文字列から戻す。`user_id` は今ログインしているユーザー。
    pub fn from_json(json: &str, user_id: &str) -> Result<Self, CryptoError> {
        let invalid = |message: String| CryptoError::InvalidTrustStore(message);
        let parsed: TrustStoreJson =
            serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
        if parsed.version != TRUST_STORE_VERSION {
            return Err(invalid(format!(
                "版 {} には対応していません",
                parsed.version
            )));
        }
        if parsed.user_id != user_id {
            return Err(invalid(format!(
                "{} の記録で、{user_id} の記録ではありません",
                parsed.user_id
            )));
        }
        validate_uuid("user_id", user_id, false)?;
        let normalize = |key: &str| {
            Ed25519PublicKey::from_base64(key)
                .map(|k| k.to_base64())
                .map_err(|e| invalid(format!("鍵 {key:?} が正しくありません: {e}")))
        };
        let own_master_key = parsed
            .own_master_key
            .as_deref()
            .map(normalize)
            .transpose()?;
        let usk_history = parsed
            .user_signing_key_history
            .iter()
            .map(|k| normalize(k))
            .collect::<Result<Vec<_>, _>>()?;
        let mut records = BTreeMap::new();
        for (target, record) in parsed.records {
            validate_uuid("user_id", &target, false)?;
            records.insert(
                target,
                TrustRecord {
                    master_key: normalize(&record.master_key)?,
                    confirmed_usk: record.confirmed_usk.as_deref().map(normalize).transpose()?,
                    pin_pending: record.pin_pending,
                    replaced_master_keys: record
                        .replaced_master_keys
                        .iter()
                        .map(|k| normalize(k))
                        .collect::<Result<Vec<_>, _>>()?,
                },
            );
        }
        Ok(Self {
            user_id: parsed.user_id,
            own_master_key,
            usk_history,
            records,
        })
    }

    /// 自分のユーザー ID。
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// 信頼している自分のマスター鍵。
    pub fn own_master_key(&self) -> Option<&str> {
        self.own_master_key.as_deref()
    }

    /// 自分のユーザー署名鍵の公開鍵の履歴（古い順。最後が今の基準）。
    pub fn user_signing_key_history(&self) -> &[String] {
        &self.usk_history
    }

    /// 相手の記録。
    pub fn record(&self, target_user_id: &str) -> Option<&TrustRecord> {
        self.records.get(target_user_id)
    }

    /// すべての相手の記録（`user_id` の順）。
    pub fn records(&self) -> impl Iterator<Item = (&String, &TrustRecord)> {
        self.records.iter()
    }

    /// 失効を行った端末が、クロス署名鍵を保存したあとに呼ぶ（#362）。
    ///
    /// 新しいマスター鍵（2〜5 のとき）を信頼し、新しいユーザー署名鍵を履歴に足し、署名し直したピンの
    /// 相手（記録があり `confirmed_usk` もある相手）の `confirmed_usk` を新しい鍵にする。
    /// 署名し直さなかった相手の記録は変えない（次の判定で再検証が必要になる）。
    ///
    /// ピンの登録待ち（`pin_pending`）は外さない。署名し直したピンの保存はこのあとなので、保存に
    /// 失敗しても配らないようにするため。保存できたら、次の判定でサーバーのピンと照合して外れる。
    pub fn apply_revocation(
        &mut self,
        master: &CrossSigningKeyPair,
        user_signing: &CrossSigningKeyPair,
        resigned: &crate::ResignPlan,
    ) -> Result<(), CryptoError> {
        self.require_own_key(master, KeyUsage::Master)?;
        self.require_own_key(user_signing, KeyUsage::UserSigning)?;
        let usk = user_signing.public_key().to_base64();
        self.push_usk(usk.clone())?;
        self.own_master_key = Some(master.public_key().to_base64());
        for pin in &resigned.pins {
            if let Some(record) = self
                .records
                .get_mut(&pin.target_user_id)
                .filter(|r| r.confirmed_usk.is_some() && r.master_key == pin.master_key)
            {
                record.confirmed_usk = Some(usk.clone());
            }
        }
        Ok(())
    }

    /// 自分のマスター鍵を信頼する。初回設定で作ったとき、または SSSS から取り出した秘密鍵を
    /// 渡す（秘密鍵を持っていることが、そのマスター鍵が自分のものだという確認になる）。
    pub fn trust_own_master_key(
        &mut self,
        master: &CrossSigningKeyPair,
    ) -> Result<(), CryptoError> {
        self.require_own_key(master, KeyUsage::Master)?;
        self.own_master_key = Some(master.public_key().to_base64());
        Ok(())
    }

    /// 自分のユーザー署名鍵を保存したとき（初回設定、解錠、失効の手順）に、履歴に足す。
    ///
    /// 一度置き換わった鍵（履歴にあって最後でない鍵）なら [`CryptoError::UserSigningKeyRollback`] を返す。
    pub fn record_own_user_signing_key(
        &mut self,
        user_signing: &CrossSigningKeyPair,
    ) -> Result<(), CryptoError> {
        self.require_own_key(user_signing, KeyUsage::UserSigning)?;
        self.push_usk(user_signing.public_key().to_base64())
    }

    /// 利用者が相手の新しいマスター鍵を受け入れた（#362）、または SAS で検証した（#368）。記録を
    /// 今のユーザー署名鍵で確認したものに置き換え、サーバーに送るピンと署名を返す。
    ///
    /// 記録は、ピンをサーバーに置けるまで登録待ち（`pin_pending`）にする。置けたら
    /// [`confirm_accepted_master_key`](Self::confirm_accepted_master_key) で外す。置けないまま判定すると、
    /// サーバーのピンがなければ TOFU と同じくピンを返し直し、違うマスター鍵のピンがあれば再検証に戻る
    /// （ピンも署名もサーバーにないのに、記録だけで配らないようにする）。
    ///
    /// `user_signing` は、履歴の最後（今の基準）のユーザー署名鍵でなければならない。
    ///
    /// 記録に別のマスター鍵があれば、その鍵を [`TrustRecord::replaced_master_keys`] に移す。受け入れた
    /// 鍵が履歴にあれば、利用者が明示的に選んだので履歴から外す。
    pub fn accept_master_key(
        &mut self,
        user_signing: &CrossSigningKeyPair,
        target_user_id: &str,
        master_key: &Value,
    ) -> Result<AcceptedMasterKey, CryptoError> {
        self.require_own_key(user_signing, KeyUsage::UserSigning)?;
        let usk = user_signing.public_key().to_base64();
        if self.usk_history.last() != Some(&usk) {
            return Err(CryptoError::StaleUserSigningKey(usk));
        }
        let signature = user_signing.sign_master_key(target_user_id, master_key)?;
        let master = verify_cross_signing_key(master_key, target_user_id, KeyUsage::Master)?;
        let pin = user_signing.sign_tofu_pin(target_user_id, &master)?;
        self.replace_record(target_user_id, master.to_base64(), Some(usk), true);
        Ok(AcceptedMasterKey {
            target_user_id: target_user_id.to_owned(),
            master_key: master.to_base64(),
            pin,
            signature,
        })
    }

    /// [`accept_master_key`](Self::accept_master_key) のピン（`replace: true`）と署名をサーバーに
    /// 保存できたときに、記録の登録待ちを外す。記録が別のマスター鍵に変わっていれば何もしない。
    pub fn confirm_accepted_master_key(&mut self, target_user_id: &str, master_key: &str) {
        if let Some(record) = self
            .records
            .get_mut(target_user_id)
            .filter(|r| r.pin_pending && r.master_key == master_key)
        {
            record.pin_pending = false;
        }
    }

    /// TOFU で作ったピンの登録（`POST /api/e2ee/tofu_pins`）が成功したときに、記録の登録待ちを外す。
    /// そのあと判定し直すと、その相手の端末を配布先に入れる。
    pub fn confirm_tofu_pin(&mut self, pin: &NewTofuPin) {
        self.confirm_accepted_master_key(&pin.target_user_id, &pin.master_key);
    }

    /// TOFU で作ったピンの登録が 409 `E2EE_PIN_CONFLICT` になったときに、登録待ちの記録を戻す。
    /// そのあと、返ったピンを [`KeySnapshot::tofu_pins`] に入れて判定し直す（共有ピンの採用）。
    /// 登録待ちでない記録は消さない。置き換えたマスター鍵の履歴がある記録（受け入れた鍵の登録待ち）も
    /// 消さない（履歴を失うと、古い鍵のピンを採用してしまうため）。その場合は、次の判定で返ったピンと
    /// 履歴を照らして扱う。
    pub fn discard_tofu_pin(&mut self, pin: &NewTofuPin) {
        if self.records.get(&pin.target_user_id).is_some_and(|r| {
            r.pin_pending && r.master_key == pin.master_key && r.replaced_master_keys.is_empty()
        }) {
            self.records.remove(&pin.target_user_id);
        }
    }

    /// 配布先を判定する（規則 1〜6）。
    ///
    /// - `snapshot` は、送信用セッションの epoch と同じ `membership_epoch` のスナップショット。
    ///   自分のマスター鍵とユーザー署名鍵も含んでいなければならない。
    /// - `own_device_id` は今の端末。自分には配らない。
    /// - `user_signing` は暗号ストアにある自分のユーザー署名鍵の秘密鍵。ないとき（解錠前）は、
    ///   初めて見た相手のピンを作れないので、その相手には配らない。
    ///
    /// 記録（TOFU の記録、`confirmed_usk`、ユーザー署名鍵の履歴）を更新するので、呼んだあとは保存する。
    /// 初めて見た相手は登録待ち（[`UserState::PinPending`]）になり、[`Distribution::new_pins`] の
    /// ピンをサーバーに置いて [`confirm_tofu_pin`](Self::confirm_tofu_pin) を呼ぶまで配らない。
    pub fn evaluate(
        &mut self,
        snapshot: &KeySnapshot,
        own_device_id: &str,
        user_signing: Option<&CrossSigningKeyPair>,
    ) -> Distribution {
        let mut out = Distribution::default();
        let me = self.user_id.clone();

        // 規則 1 の前提：自分のマスター鍵を信頼できるか。
        let own_master = self.check_own_master(snapshot, &mut out.warnings);
        // ユーザー署名鍵の基準：自分のマスター鍵で署名され、巻き戻しでないもの。
        let usk = own_master.and_then(|master| self.check_own_usk(snapshot, &master, &mut out));
        let usk_secret = user_signing.filter(|k| {
            k.usage() == KeyUsage::UserSigning
                && k.user_id() == me
                && usk.is_some_and(|usk| k.public_key() == usk)
        });

        let mut user_ids = snapshot.user_ids.clone();
        user_ids.sort();
        user_ids.dedup();
        for user_id in user_ids {
            let trust = if user_id == me {
                match own_master {
                    Some(master) => self.trusted_devices(
                        snapshot,
                        &user_id,
                        &master,
                        Some(own_device_id),
                        TrustBasis::Own,
                        &mut out.devices,
                    ),
                    None => untrusted(UserState::Blocked),
                }
            } else {
                self.evaluate_other(snapshot, &user_id, usk, usk_secret, &mut out)
            };
            out.users.insert(user_id, trust);
        }
        out
    }

    /// 受信時の検証の手順 4 に渡す、送信ユーザーの信頼した自己署名鍵（docs/e2ee.md の「受信時の検証」）。
    ///
    /// マスター鍵を信頼するかは、配布先の判定と同じ規則で決める（自分は規則 1、他のユーザーは
    /// 規則 3〜6）。信頼したマスター鍵で署名された自己署名鍵と、`snapshot.signatures` にある
    /// その自己署名鍵による `sender_device` への署名を返す。次のときは `None`。
    ///
    /// - 自分のマスター鍵をまだ信頼していない（未解錠）か、サーバーの自分のマスター鍵が変わった。
    /// - 送信ユーザーが自分で、サーバーの自己署名鍵が `own_self_signing`（暗号ストアの自分の
    ///   自己署名鍵。SSSS から取り出した今の鍵）と違う、または `own_self_signing` がない。失効で
    ///   置き換えた古い自己署名鍵（マスター鍵の署名は残る）をサーバーが返しても、外した端末を
    ///   検証済みにしないため。
    /// - 送信ユーザーのマスター鍵を信頼できない（クロス署名鍵がない、再検証待ち、TOFU のピンの登録待ち、
    ///   初めて見た相手）。初めて見た相手のピンは作らない。作るのは配布先の判定だけ。
    /// - 自己署名鍵がない、壊れている、マスター鍵で署名されていない。
    ///
    /// `snapshot` は送信ユーザーと自分の `keys/query`（他のユーザーを判定するには、自分のマスター鍵と
    /// ユーザー署名鍵が要る）。規則 3 や共有ピンの採用で記録が変わることがあるので、呼んだあとは保存する。
    pub fn sender_trust(
        &mut self,
        snapshot: &KeySnapshot,
        sender: &str,
        sender_device: &str,
        own_self_signing: Option<&Ed25519PublicKey>,
    ) -> Option<SenderTrust> {
        let mut scratch = Distribution::default();
        let own_master = self.check_own_master(snapshot, &mut scratch.warnings)?;
        let master = if sender == self.user_id {
            own_master
        } else {
            let usk = self.check_own_usk(snapshot, &own_master, &mut scratch);
            let trust = self.evaluate_other(snapshot, sender, usk, None, &mut scratch);
            if !matches!(trust.state, UserState::Trusted(_)) {
                return None;
            }
            verify_cross_signing_key(snapshot.master_keys.get(sender)?, sender, KeyUsage::Master)
                .ok()?
        };
        let self_signing = trusted_self_signing_key(snapshot, sender, &master)?;
        if sender == self.user_id && own_self_signing != Some(&self_signing) {
            return None;
        }
        let key_id = cross_signing_key_id(&self_signing);
        let signature = snapshot
            .signatures
            .iter()
            .find(|row| {
                row.signer_user_id == sender
                    && row.signer_key_id == key_id
                    && row.target_user_id == sender
                    && row.target_key_id == sender_device
            })
            .map(|row| row.signature.clone());
        Some(SenderTrust {
            self_signing_key: self_signing,
            signature,
        })
    }

    /// 他のユーザー（規則 2〜6）。
    fn evaluate_other(
        &mut self,
        snapshot: &KeySnapshot,
        user_id: &str,
        usk: Option<Ed25519PublicKey>,
        usk_secret: Option<&CrossSigningKeyPair>,
        out: &mut Distribution,
    ) -> UserTrust {
        // 規則 2：クロス署名鍵がない。
        let Some(master_json) = snapshot.master_keys.get(user_id) else {
            return untrusted(UserState::NoCrossSigningKeys);
        };
        let Ok(master) = verify_cross_signing_key(master_json, user_id, KeyUsage::Master) else {
            return untrusted(UserState::InvalidCrossSigningKeys);
        };
        let Some(usk) = usk else {
            return untrusted(UserState::Blocked);
        };
        let master_b64 = master.to_base64();
        let usk_b64 = usk.to_base64();
        let me = self.user_id.clone();

        // 置き換えた古いマスター鍵への巻き戻し：署名やピンがあっても信頼しない（規則 3 より先に見る）。
        if self
            .records
            .get(user_id)
            .is_some_and(|r| r.replaced_master_keys.contains(&master_b64))
        {
            return untrusted(UserState::NeedsReverification(
                ReverificationReason::MasterKeyChanged,
            ));
        }

        // 規則 3：自分のユーザー署名鍵で署名されている。
        let signed = check_signature(
            master_json,
            user_id,
            &master_b64,
            &me,
            &usk,
            &snapshot.signatures,
        ) == SignatureCheck::Valid;
        if signed {
            // 記録と違う鍵なら、古い鍵を履歴に移し、置き換えたことを知らせる。
            if self
                .records
                .get(user_id)
                .is_some_and(|r| r.master_key != master_b64)
            {
                out.signed_master_key_changes.push(user_id.to_owned());
            }
            self.replace_record(user_id, master_b64, Some(usk_b64), false);
            return self.trusted_devices(
                snapshot,
                user_id,
                &master,
                None,
                TrustBasis::UserSigningSignature,
                &mut out.devices,
            );
        }

        // 今のユーザー署名鍵で検証できるピンだけを使う。
        let pin = snapshot
            .tofu_pins
            .get(user_id)
            .and_then(|pin| verified_pin(pin, &me, user_id, &usk));

        // 共有ピンを採用するときに引き継ぐ、置き換えたマスター鍵の履歴。
        let mut carried_history = Vec::new();
        // 登録待ちの TOFU の記録。
        if let Some(record) = self.records.get(user_id).filter(|r| r.pin_pending).cloned() {
            match &pin {
                // サーバーに同じマスター鍵のピンがある：登録できていたので、ふつうの記録として扱う。
                Some(pinned) if *pinned == record.master_key => {
                    if let Some(r) = self.records.get_mut(user_id) {
                        r.pin_pending = false;
                    }
                }
                // サーバーのピンが、置き換えた古いマスター鍵のもの：巻き戻しなので採用しない。
                Some(pinned) if record.replaced_master_keys.contains(pinned) => {
                    return untrusted(UserState::NeedsReverification(
                        ReverificationReason::MasterKeyChanged,
                    ));
                }
                // 別の端末が違うマスター鍵のピンを先に作っていた（409 と同じ）：記録を戻し、
                // そのピンを共有ピンとして採用する（下の記録がない場合）。履歴は引き継ぎ、登録待ちだった
                // 鍵も履歴に足す（同じ相手に違う鍵を見せられたので、黙って戻さない）。
                Some(_) => {
                    if let Some(removed) = self.records.remove(user_id) {
                        carried_history = removed.replaced_master_keys;
                        push_unique(&mut carried_history, removed.master_key);
                    }
                }
                // まだ登録できていない：同じピンを作り直して返し、配らない。
                None => {
                    if record.master_key != master_b64 {
                        return untrusted(UserState::NeedsReverification(
                            ReverificationReason::MasterKeyChanged,
                        ));
                    }
                    if record.confirmed_usk.as_deref() != Some(usk_b64.as_str()) {
                        return untrusted(UserState::NeedsReverification(
                            ReverificationReason::UserSigningKeyChanged,
                        ));
                    }
                    let Some(Ok(pin)) = usk_secret.map(|k| k.sign_tofu_pin(user_id, &master))
                    else {
                        return untrusted(UserState::NeedsUnlock);
                    };
                    out.new_pins.push(NewTofuPin {
                        target_user_id: user_id.to_owned(),
                        master_key: master_b64,
                        pin,
                    });
                    return untrusted(UserState::PinPending);
                }
            }
        }

        let basis = match self.records.get(user_id).cloned() {
            None => match pin {
                // 共有ピンの採用。マスター鍵が今のものと一致するときだけ confirmed_usk を設定する。
                Some(pinned) => {
                    let matches = pinned == master_b64;
                    self.records.insert(
                        user_id.to_owned(),
                        TrustRecord {
                            master_key: pinned,
                            confirmed_usk: matches.then_some(usk_b64),
                            pin_pending: false,
                            replaced_master_keys: carried_history,
                        },
                    );
                    if !matches {
                        return untrusted(UserState::NeedsReverification(
                            ReverificationReason::MasterKeyChanged,
                        ));
                    }
                    TrustBasis::SharedPin
                }
                // 規則 4：初めて見た。TOFU でピンと登録待ちの記録を作る。ピンをサーバーに置けて
                // （別の端末のピンとの競合がないと確かめて）から信頼する。
                None => {
                    let Some(secret) = usk_secret else {
                        return untrusted(UserState::NeedsUnlock);
                    };
                    // user_id が UUID でなければピンを作れないので、壊れた鍵として扱う。
                    let Ok(pin) = secret.sign_tofu_pin(user_id, &master) else {
                        return untrusted(UserState::InvalidCrossSigningKeys);
                    };
                    out.new_pins.push(NewTofuPin {
                        target_user_id: user_id.to_owned(),
                        master_key: master_b64.clone(),
                        pin,
                    });
                    self.records.insert(
                        user_id.to_owned(),
                        TrustRecord {
                            master_key: master_b64,
                            confirmed_usk: Some(usk_b64),
                            pin_pending: true,
                            replaced_master_keys: carried_history,
                        },
                    );
                    return untrusted(UserState::PinPending);
                }
            },
            // 規則 5：記録かピンのマスター鍵が違う、confirmed_usk がない、
            // またはユーザー署名鍵が変わって署名し直されたピンを確かめられない。
            Some(record) => {
                if record.master_key != master_b64 || pin.as_ref().is_some_and(|p| *p != master_b64)
                {
                    return untrusted(UserState::NeedsReverification(
                        ReverificationReason::MasterKeyChanged,
                    ));
                }
                let Some(confirmed) = record.confirmed_usk else {
                    return untrusted(UserState::NeedsReverification(
                        ReverificationReason::NotConfirmed,
                    ));
                };
                if confirmed != usk_b64 {
                    // 署名し直されたピン（今の鍵で検証でき、記録と同じマスター鍵）があれば更新する。
                    if pin.as_deref() != Some(record.master_key.as_str()) {
                        return untrusted(UserState::NeedsReverification(
                            ReverificationReason::UserSigningKeyChanged,
                        ));
                    }
                    if let Some(r) = self.records.get_mut(user_id) {
                        r.confirmed_usk = Some(usk_b64);
                        r.pin_pending = false;
                    }
                }
                TrustBasis::Record
            }
        };
        // 規則 6。
        self.trusted_devices(snapshot, user_id, &master, None, basis, &mut out.devices)
    }

    /// 規則 1 と 6：信頼したマスター鍵から、自己署名鍵を経て署名をたどれる端末を集める。
    fn trusted_devices(
        &self,
        snapshot: &KeySnapshot,
        user_id: &str,
        master: &Ed25519PublicKey,
        skip_device: Option<&str>,
        basis: TrustBasis,
        devices: &mut Vec<DeviceKeys>,
    ) -> UserTrust {
        let mut excluded = BTreeMap::new();
        let self_signing = trusted_self_signing_key(snapshot, user_id, master);
        for (device_id, json) in snapshot.device_keys.get(user_id).into_iter().flatten() {
            let reason = if skip_device == Some(device_id.as_str()) {
                Some(DeviceExclusion::CurrentDevice)
            } else if let Some(ssk) = &self_signing {
                match verify_device_keys(json, user_id, device_id) {
                    Err(_) => Some(DeviceExclusion::InvalidDeviceKeys),
                    Ok(keys) => match check_signature(
                        json,
                        user_id,
                        device_id,
                        user_id,
                        ssk,
                        &snapshot.signatures,
                    ) {
                        SignatureCheck::Valid => {
                            devices.push(keys);
                            None
                        }
                        SignatureCheck::Missing => Some(DeviceExclusion::Unsigned),
                        SignatureCheck::Invalid => Some(DeviceExclusion::BadSignature),
                    },
                }
            } else {
                Some(DeviceExclusion::InvalidSelfSigningKey)
            };
            if let Some(reason) = reason {
                excluded.insert(device_id.clone(), reason);
            }
        }
        UserTrust {
            state: UserState::Trusted(basis),
            excluded_devices: excluded,
        }
    }

    /// サーバーの自分のマスター鍵が、信頼したものと同じか。
    fn check_own_master(
        &self,
        snapshot: &KeySnapshot,
        warnings: &mut Vec<TrustWarning>,
    ) -> Option<Ed25519PublicKey> {
        let Some(trusted) = &self.own_master_key else {
            warnings.push(TrustWarning::OwnIdentityUnknown);
            return None;
        };
        let server = snapshot
            .master_keys
            .get(&self.user_id)
            .and_then(|json| verify_cross_signing_key(json, &self.user_id, KeyUsage::Master).ok());
        match server {
            Some(key) if key.to_base64() == *trusted => Some(key),
            _ => {
                warnings.push(TrustWarning::OwnMasterKeyChanged);
                None
            }
        }
    }

    /// 自分のユーザー署名鍵を、自分のマスター鍵の署名と履歴で確かめ、基準を返す。
    fn check_own_usk(
        &mut self,
        snapshot: &KeySnapshot,
        master: &Ed25519PublicKey,
        out: &mut Distribution,
    ) -> Option<Ed25519PublicKey> {
        let usk = snapshot
            .user_signing_keys
            .get(&self.user_id)
            .and_then(|json| {
                let key =
                    verify_cross_signing_key(json, &self.user_id, KeyUsage::UserSigning).ok()?;
                verify_json_signature(json, &self.user_id, &cross_signing_key_id(master), master)
                    .ok()
                    .map(|()| key)
            });
        let Some(usk) = usk else {
            out.warnings.push(TrustWarning::UntrustedUserSigningKey);
            return None;
        };
        let b64 = usk.to_base64();
        match self.usk_history.iter().position(|k| *k == b64) {
            Some(i) if i + 1 == self.usk_history.len() => {}
            Some(_) => {
                out.warnings.push(TrustWarning::UserSigningKeyRollback);
                return None;
            }
            None => {
                if !self.usk_history.is_empty() {
                    out.warnings.push(TrustWarning::UserSigningKeyChanged);
                }
                self.usk_history.push(b64);
            }
        }
        Some(usk)
    }

    /// 相手の記録を新しいマスター鍵の記録にする。前の記録のマスター鍵が違えば履歴に移し、新しい鍵が
    /// 履歴にあれば外す（呼び出し側は、履歴にある鍵を利用者の明示の受け入れなしに渡さない）。
    fn replace_record(
        &mut self,
        user_id: &str,
        master_key: String,
        confirmed_usk: Option<String>,
        pin_pending: bool,
    ) {
        let mut history = Vec::new();
        if let Some(old) = self.records.remove(user_id) {
            history = old.replaced_master_keys;
            if old.master_key != master_key {
                push_unique(&mut history, old.master_key);
            }
        }
        history.retain(|k| *k != master_key);
        self.records.insert(
            user_id.to_owned(),
            TrustRecord {
                master_key,
                confirmed_usk,
                pin_pending,
                replaced_master_keys: history,
            },
        );
    }

    fn push_usk(&mut self, key: String) -> Result<(), CryptoError> {
        match self.usk_history.iter().position(|k| *k == key) {
            Some(i) if i + 1 == self.usk_history.len() => Ok(()),
            Some(_) => Err(CryptoError::UserSigningKeyRollback(key)),
            None => {
                self.usk_history.push(key);
                Ok(())
            }
        }
    }

    fn require_own_key(
        &self,
        key: &CrossSigningKeyPair,
        usage: KeyUsage,
    ) -> Result<(), CryptoError> {
        if key.usage() != usage || key.user_id() != self.user_id {
            return Err(CryptoError::InvalidCrossSigningKey(format!(
                "{} の {} の鍵が必要です",
                self.user_id,
                usage.as_str()
            )));
        }
        Ok(())
    }
}

fn push_unique(list: &mut Vec<String>, key: String) {
    if !list.contains(&key) {
        list.push(key);
    }
}

/// `user_id` の自己署名鍵のうち、信頼したマスター鍵 `master` で署名されたもの（規則 1、6）。
fn trusted_self_signing_key(
    snapshot: &KeySnapshot,
    user_id: &str,
    master: &Ed25519PublicKey,
) -> Option<Ed25519PublicKey> {
    let json = snapshot.self_signing_keys.get(user_id)?;
    let key = verify_cross_signing_key(json, user_id, KeyUsage::SelfSigning).ok()?;
    (check_signature(
        json,
        user_id,
        &key.to_base64(),
        user_id,
        master,
        &snapshot.signatures,
    ) == SignatureCheck::Valid)
        .then_some(key)
}

fn untrusted(state: UserState) -> UserTrust {
    UserTrust {
        state,
        excluded_devices: BTreeMap::new(),
    }
}

/// 今のユーザー署名鍵で署名を検証できるピンなら、そのマスター鍵（正規化した base64）を返す。
pub(crate) fn verified_pin(
    pin: &Value,
    me: &str,
    target: &str,
    usk: &Ed25519PublicKey,
) -> Option<String> {
    let field = |name: &str| pin.get(name).and_then(Value::as_str);
    if field("type") != Some(TOFU_PIN_TYPE)
        || field("user_id") != Some(me)
        || field("target_user_id") != Some(target)
    {
        return None;
    }
    let master = Ed25519PublicKey::from_base64(field("master_key")?).ok()?;
    verify_json_signature(pin, me, &cross_signing_key_id(usk), usk).ok()?;
    Some(master.to_base64())
}
