//! 失効の手順（docs/e2ee.md の「失効の手順」。#362）。
//!
//! I/O は持たず、サーバーに送る要求の本文と、暗号ストアに置く進み具合（[`RevocationState`]）を作る。
//! 呼び出し側（Web、Android、TUI）は、次の順に送る。
//!
//! 1. 選んだ端末の削除（`DELETE /api/e2ee/devices/{device_id}`）。この crate は関わらない。
//! 2. [`Revocation::begin`] で、署名し直す相手を集め（手順 4 の【決定】）、新しい鍵を作る。
//!    返った [`RevocationState`] を暗号ストアに保存してから、サーバーに保存を始める。
//! 3. SSSS（[`Revocation::secret_storage_request`]）、クロス署名鍵
//!    （[`Revocation::cross_signing_request`]）、署名とピン（[`Revocation::signatures`]）、
//!    鍵バックアップの版（[`Revocation::backup_version_request`]）の順に保存する。
//! 4. 古い版のセッションを新しい版に移し（[`Revocation::reencrypt_rows`]）、古い版を消す。
//! 5. 読み直して確かめる（[`Revocation::verify`]）。
//!
//! 途中で止まったら、SSSS を開け直して [`Revocation::resume`] で続ける。マスター鍵の秘密鍵は
//! 暗号ストアに置かないので、SSSS を保存する前（[`RevocationStage::Prepared`]）に止まったら、
//! 最初からやり直す（端末だけの失効は同じ SSSS 鍵で開け直せるので、保存した鍵で続けられる）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vodozemac::{Ed25519PublicKey, base64_encode};
use zeroize::Zeroizing;

use crate::backup::{
    BackupDecryptionKey, BackupEncryptionKey, BackupRow, BackupVersionInfo,
    select_previous_backup_keys,
};
use crate::cross_signing::{
    CrossSigningKeyPair, CrossSigningKeys, KeySignature, KeyUsage, SignatureCheck, SignatureUpload,
    check_signature, cross_signing_key_id, verify_cross_signing_key,
};
use crate::device::{DeviceAccount, validate_uuid};
use crate::secret_storage::{
    EncryptedSecret, KeySlot, SecretStorage, SecretStorageKeysRequest, SsssKey,
    UnlockedSecretStorage, decode_fixed,
};
use crate::setup::{OwnSecrets, ServerKeyState};
use crate::store::{self, StoreKind};
use crate::trust::{TrustStore, verified_pin};
use crate::{CryptoError, SECRET_MEGOLM_BACKUP, SECRET_PREVIOUS_MEGOLM_BACKUP};

/// 保存形式の版。形式を変えたら上げる。
const REVOCATION_STATE_VERSION: u32 = 1;

/// 何が漏れた可能性があるかで決まる、失効の範囲（docs/e2ee.md の失効の表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationScope {
    /// 端末だけ（1、3、4 と SSSS の秘密の置き換え）。マスター鍵と SSSS 鍵は残す。
    DeviceOnly,
    /// パスキー、リカバリーキー、SSSS 鍵のどれか（1〜5 のすべて）。マスター鍵と SSSS 鍵も作り直す。
    Full,
}

/// どこまで保存したか。保存するたびに進め、暗号ストアに記録し直す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationStage {
    /// 署名し直す一覧を集めて記録した。サーバーにはまだ何も保存していない。
    Prepared,
    /// SSSS を保存した（2〜5 は新しい SSSS 鍵、端末だけは秘密の置き換え）。
    SecretStorageSaved,
    /// クロス署名鍵を保存した。
    CrossSigningSaved,
    /// 今の端末への署名、ピン、マスター鍵への署名を保存した。
    SignaturesSaved,
    /// 新しい鍵バックアップの版を作った。
    BackupCreated,
    /// 古い版のセッションを新しい版に移し、古い版を消した。
    Migrated,
    /// 読み直して、自分が作った鍵と一致することを確かめた。
    Verified,
}

/// 署名し直さなかった（または記録で置き換えた）理由。失効の画面に、相手ごとに出す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResignNote {
    /// 失効を行う端末に記録がない。ピンも署名も署名し直さない（再検証か SAS で作り直す）。
    NoRecord,
    /// 記録はあるが `confirmed_usk` がない（共有ピンから採用しただけの記録など）。署名し直さない。
    NotConfirmed,
    /// サーバーのピンが、古いユーザー署名鍵で検証できないか、記録と違うマスター鍵だった。
    /// 記録のマスター鍵のピンで置き換える。
    PinReplaced,
    /// マスター鍵への署名を、暗号ストアの古いユーザー署名鍵で検証できなかった。署名し直さない。
    SignatureUnverified,
    /// マスター鍵への署名は検証できたが、記録と違うマスター鍵への署名だった。署名し直さない。
    MasterKeyMismatch,
}

/// 新しいユーザー署名鍵で作り直すピン（記録のマスター鍵）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResignPin {
    /// 相手。
    pub target_user_id: String,
    /// 記録のマスター鍵（パディングなしの base64）。
    pub master_key: String,
}

/// 新しいユーザー署名鍵で署名し直すマスター鍵。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResignMasterKey {
    /// 相手。
    pub target_user_id: String,
    /// `keys/query` で取った相手の今のマスター鍵の JSON。
    pub master_key: Value,
}

/// 失効の手順 4 で、署名し直すものと、失効の画面に出す一覧（docs/e2ee.md の手順 4 の【決定】）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResignPlan {
    /// 作り直すピン（`replace: true`）。記録があり `confirmed_usk` もある相手。
    pub pins: Vec<ResignPin>,
    /// 署名し直すマスター鍵。古いユーザー署名鍵で検証でき、記録と同じマスター鍵へのもの。
    pub master_keys: Vec<ResignMasterKey>,
    /// 失効の画面に出す相手と理由。
    pub notes: BTreeMap<String, BTreeSet<ResignNote>>,
}

impl ResignPlan {
    /// 失効の手順 4 の候補を集め、失効を行う端末の記録に絞る。
    ///
    /// - `old_user_signing` は暗号ストアにある古いユーザー署名鍵（秘密鍵）。ピンと署名は、
    ///   この鍵から導いた公開鍵だけで検証する（サーバーが見せる鍵は使わない）。
    /// - `pins` は `GET /api/e2ee/tofu_pins` で（`next` が `null` になるまで）取ったすべてのピン。
    /// - `signatures` と `master_keys` は、ピンか記録がある相手を `keys/query` で問い合わせた応答の
    ///   `signatures` と `master_keys`。
    pub fn collect(
        trust: &TrustStore,
        old_user_signing: &CrossSigningKeyPair,
        pins: &[Value],
        signatures: &[KeySignature],
        master_keys: &BTreeMap<String, Value>,
    ) -> Result<Self, CryptoError> {
        let me = trust.user_id();
        if old_user_signing.usage() != KeyUsage::UserSigning || old_user_signing.user_id() != me {
            return Err(CryptoError::InvalidCrossSigningKey(format!(
                "{me} のユーザー署名鍵が必要です"
            )));
        }
        let old_usk = old_user_signing.public_key();
        let old_key_id = cross_signing_key_id(&old_usk);
        let mut plan = Self::default();
        fn note(plan: &mut ResignPlan, user: &str, n: ResignNote) {
            plan.notes.entry(user.to_owned()).or_default().insert(n);
        }

        // サーバーのピンを相手ごとに読む（古いユーザー署名鍵で検証できたマスター鍵か、検証できない）。
        let mut server_pins: BTreeMap<String, Option<String>> = BTreeMap::new();
        for pin in pins {
            let Some(target) = pin.get("target_user_id").and_then(Value::as_str) else {
                continue;
            };
            if validate_uuid("target_user_id", target, false).is_err() || target == me {
                continue;
            }
            server_pins.insert(target.to_owned(), verified_pin(pin, me, target, &old_usk));
        }

        for (target, record) in trust.records() {
            if record.confirmed_usk.is_none() {
                note(&mut plan, target, ResignNote::NotConfirmed);
                continue;
            }
            plan.pins.push(ResignPin {
                target_user_id: target.clone(),
                master_key: record.master_key.clone(),
            });
            if let Some(verified) = server_pins.get(target)
                && verified.as_deref() != Some(record.master_key.as_str())
            {
                note(&mut plan, target, ResignNote::PinReplaced);
            }
        }
        for target in server_pins.keys() {
            if trust.record(target).is_none() {
                note(&mut plan, target, ResignNote::NoRecord);
            }
        }

        let mut signed: BTreeSet<String> = BTreeSet::new();
        for row in signatures
            .iter()
            .filter(|row| row.signer_user_id == me && row.target_user_id != me)
        {
            let target = row.target_user_id.as_str();
            if validate_uuid("target_user_id", target, false).is_err() {
                continue;
            }
            let master = master_keys.get(target).and_then(|json| {
                verify_cross_signing_key(json, target, KeyUsage::Master)
                    .ok()
                    .filter(|key| key.to_base64() == row.target_key_id)
                    .map(|key| (json, key))
            });
            let valid = row.signer_key_id == old_key_id
                && master.is_some_and(|(json, key)| {
                    check_signature(
                        json,
                        target,
                        &key.to_base64(),
                        me,
                        &old_usk,
                        std::slice::from_ref(row),
                    ) == SignatureCheck::Valid
                });
            let Some((json, key)) = master.filter(|_| valid) else {
                note(&mut plan, target, ResignNote::SignatureUnverified);
                continue;
            };
            match trust.record(target) {
                None => note(&mut plan, target, ResignNote::NoRecord),
                Some(record) if record.confirmed_usk.is_none() => {
                    note(&mut plan, target, ResignNote::NotConfirmed)
                }
                Some(record) if record.master_key != key.to_base64() => {
                    note(&mut plan, target, ResignNote::MasterKeyMismatch)
                }
                Some(_) => {
                    if signed.insert(target.to_owned()) {
                        plan.master_keys.push(ResignMasterKey {
                            target_user_id: target.to_owned(),
                            master_key: json.clone(),
                        });
                    }
                }
            }
        }
        Ok(plan)
    }

    /// 新しいユーザー署名鍵で、ピン（`replace: true` で送る）とマスター鍵への署名を作る。
    pub fn sign(
        &self,
        new_user_signing: &CrossSigningKeyPair,
    ) -> Result<(Vec<Value>, Vec<SignatureUpload>), CryptoError> {
        let pins = self
            .pins
            .iter()
            .map(|pin| {
                let master = Ed25519PublicKey::from_base64(&pin.master_key)?;
                new_user_signing.sign_tofu_pin(&pin.target_user_id, &master)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let signatures = self
            .master_keys
            .iter()
            .map(|entry| new_user_signing.sign_master_key(&entry.target_user_id, &entry.master_key))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((pins, signatures))
    }
}

/// クロス署名鍵の公開鍵 3 つ（`expected_keys` と、作った鍵の照合に使う）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCrossSigningKeys {
    /// マスター鍵。
    pub master: Option<String>,
    /// 自己署名鍵。
    pub self_signing: Option<String>,
    /// ユーザー署名鍵。
    pub user_signing: Option<String>,
}

impl PublicCrossSigningKeys {
    fn of(keys: &CrossSigningKeys) -> Self {
        Self {
            master: Some(keys.master.public_key().to_base64()),
            self_signing: Some(keys.self_signing.public_key().to_base64()),
            user_signing: Some(keys.user_signing.public_key().to_base64()),
        }
    }
}

/// 暗号ストアに置く失効の進み具合（docs/e2ee.md の「どこまで保存したかを暗号ストアに記録し」）。
///
/// 新しい自己署名鍵、ユーザー署名鍵、バックアップ鍵の秘密鍵と、古いバックアップ鍵を含む。
/// マスター鍵の秘密鍵と SSSS 鍵は含まない。[`pickle`](Self::pickle) で封筒に入れて保存する。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationState {
    version: u32,
    user_id: String,
    /// 範囲。
    pub scope: RevocationScope,
    /// どこまで保存したか。
    pub stage: RevocationStage,
    /// 削除した端末（表示用）。
    pub deleted_devices: Vec<String>,
    /// 始める前のサーバーのクロス署名鍵（`expected_keys`）。
    pub expected_keys: PublicCrossSigningKeys,
    /// 始める前の既定の SSSS 鍵。
    pub expected_default_key_id: String,
    /// 始める前の `secrets_revision`。
    pub expected_secrets_revision: Option<u64>,
    /// 始める前の今の版（`expected_current_version`）。
    pub expected_current_version: Option<String>,
    /// 新しい既定の SSSS 鍵（2〜5）。端末だけの失効では、始める前の鍵と同じ。
    pub key_id: String,
    /// 作った鍵の公開鍵。
    pub new_keys: PublicCrossSigningKeys,
    /// 作ったバックアップ鍵の公開鍵。
    pub new_backup_public_key: String,
    new_self_signing: String,
    new_user_signing: String,
    new_backup_key: String,
    /// `zoo.megolm_backup.previous` に入れた古いバックアップ鍵（秘密鍵）。古い版を消すまで残す。
    previous_backup_keys: Vec<String>,
    /// 消していない版のうち、秘密鍵がどの候補にもない版（「新しい版に移せないバックアップ」）。
    pub versions_without_key: Vec<String>,
    /// 署名し直すものと、失効の画面に出す一覧。
    pub resign: ResignPlan,
    /// SSSS を保存した応答の `secrets_revision`。
    pub secrets_revision: Option<u64>,
    /// 作った新しい版。
    pub new_backup_version: Option<String>,
}

impl std::fmt::Debug for RevocationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevocationState")
            .field("scope", &self.scope)
            .field("stage", &self.stage)
            .field("key_id", &self.key_id)
            .field("new_keys", &self.new_keys)
            .field("new_backup_version", &self.new_backup_version)
            .finish_non_exhaustive()
    }
}

impl Drop for RevocationState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.new_self_signing.zeroize();
        self.new_user_signing.zeroize();
        self.new_backup_key.zeroize();
        for key in &mut self.previous_backup_keys {
            key.zeroize();
        }
    }
}

impl RevocationState {
    /// 暗号ストアの封筒（`kind` は `revocation`）に入れる。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8; 32]) -> String {
        let json = Zeroizing::new(
            serde_json::to_string(self).expect("文字列と数値だけの構造体は必ず JSON にできる"),
        );
        store::seal(storage_key, StoreKind::Revocation, account, json.as_bytes())
    }

    /// [`pickle`](Self::pickle) の値を戻す。
    pub fn from_pickle(
        sealed: &str,
        account: &DeviceAccount,
        storage_key: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        let plaintext = store::open(sealed, storage_key, StoreKind::Revocation, account)?;
        let state: Self = serde_json::from_slice(&plaintext)
            .map_err(|_| CryptoError::InvalidPickle("失効の記録の JSON を読めません".into()))?;
        if state.version != REVOCATION_STATE_VERSION || state.user_id != account.user_id() {
            return Err(CryptoError::InvalidPickle(
                "失効の記録の版かユーザーが違います".into(),
            ));
        }
        Ok(state)
    }

    /// 古い版のバックアップ鍵（移行に使う）。
    pub fn previous_backup_keys(&self) -> Result<Vec<BackupDecryptionKey>, CryptoError> {
        self.previous_backup_keys
            .iter()
            .map(|key| backup_key_from_base64(key))
            .collect()
    }

    /// 作った自己署名鍵、ユーザー署名鍵、バックアップ鍵。端末だけの失効の再開と、
    /// 保存のあとの暗号ストアの更新に使う。
    pub fn new_subkeys(
        &self,
    ) -> Result<
        (
            CrossSigningKeyPair,
            CrossSigningKeyPair,
            BackupDecryptionKey,
        ),
        CryptoError,
    > {
        let seed = |name: &str, value: &str| {
            decode_fixed::<32>(name, value)
                .map(Zeroizing::new)
                .map_err(|_| CryptoError::InvalidPickle("失効の記録の秘密鍵が壊れています".into()))
        };
        Ok((
            CrossSigningKeyPair::from_seed(
                &self.user_id,
                KeyUsage::SelfSigning,
                &*seed("self_signing", &self.new_self_signing)?,
            )?,
            CrossSigningKeyPair::from_seed(
                &self.user_id,
                KeyUsage::UserSigning,
                &*seed("user_signing", &self.new_user_signing)?,
            )?,
            backup_key_from_base64(&self.new_backup_key)?,
        ))
    }
}

fn backup_key_from_base64(value: &str) -> Result<BackupDecryptionKey, CryptoError> {
    let bytes = Zeroizing::new(
        decode_fixed::<32>("backup_key", value)
            .map_err(|_| CryptoError::InvalidPickle("バックアップ鍵が壊れています".into()))?,
    );
    Ok(BackupDecryptionKey::from_bytes(&bytes))
}

/// [`Revocation::begin`] に渡すもの。
#[derive(Debug)]
pub struct RevocationInput<'a> {
    /// 範囲。
    pub scope: RevocationScope,
    /// 始める前に開け直した SSSS。
    pub unlocked: &'a UnlockedSecretStorage,
    /// 失効を行う端末の記録。
    pub trust: &'a TrustStore,
    /// サーバーの自分の鍵と、すべての版（`keys/query` と `GET /api/e2ee/key_backup/versions`）。
    pub server: &'a ServerKeyState,
    /// `GET /api/e2ee/tofu_pins` のすべてのピン。
    pub pins: &'a [Value],
    /// ピンか記録がある相手の `keys/query` の `signatures`。
    pub signatures: &'a [KeySignature],
    /// ピンか記録がある相手の `keys/query` の `master_keys`。
    pub master_keys: &'a BTreeMap<String, Value>,
    /// 失効を行う端末の暗号ストアにあるバックアップ鍵（あれば）。
    pub local_backup_key: Option<&'a BackupDecryptionKey>,
    /// 手順 1 で削除した端末。
    pub deleted_devices: Vec<String>,
}

/// 進めている失効。マスター鍵の秘密鍵と SSSS 鍵をメモリーに持つので、終わったら捨てる。
#[derive(Debug)]
pub struct Revocation {
    state: RevocationState,
    secrets: OwnSecrets,
    ssss: SsssKey,
    previous: Vec<BackupDecryptionKey>,
}

impl Revocation {
    /// 失効を始める。署名し直す一覧を集め、新しい鍵を作る。返した [`state`](Self::state) を
    /// 暗号ストアに保存してから、サーバーへの保存を始める。
    ///
    /// サーバーのクロス署名鍵が、開け直した SSSS の鍵と違えば [`CryptoError::ServerKeyMismatch`]
    /// （別の端末の失効が入り混じった。SSSS を開け直してやり直す）。
    pub fn begin(input: RevocationInput<'_>) -> Result<Self, CryptoError> {
        let old = OwnSecrets::from_unlocked(input.unlocked)?;
        let user_id = old.cross_signing.master.user_id().to_owned();
        if input.trust.user_id() != user_id {
            return Err(CryptoError::InvalidTrustStore(
                "記録が SSSS のユーザーのものではありません".into(),
            ));
        }
        let mut expected_keys = PublicCrossSigningKeys::default();
        for (slot, pair) in [
            (&mut expected_keys.master, &old.cross_signing.master),
            (
                &mut expected_keys.self_signing,
                &old.cross_signing.self_signing,
            ),
            (
                &mut expected_keys.user_signing,
                &old.cross_signing.user_signing,
            ),
        ] {
            let on_server = server_public_key(input.server, &user_id, pair.usage())?;
            if on_server.is_some_and(|key| key != pair.public_key()) {
                return Err(CryptoError::ServerKeyMismatch(format!(
                    "サーバーの {} が SSSS の鍵と違います",
                    pair.usage().as_str()
                )));
            }
            *slot = on_server.map(|key| key.to_base64());
        }

        let resign = ResignPlan::collect(
            input.trust,
            &old.cross_signing.user_signing,
            input.pins,
            input.signatures,
            input.master_keys,
        )?;

        let master = match input.scope {
            RevocationScope::Full => CrossSigningKeyPair::generate(&user_id, KeyUsage::Master)?,
            RevocationScope::DeviceOnly => CrossSigningKeyPair::from_seed(
                &user_id,
                KeyUsage::Master,
                &old.cross_signing.master.seed(),
            )?,
        };
        let secrets = OwnSecrets {
            cross_signing: CrossSigningKeys {
                master,
                self_signing: CrossSigningKeyPair::generate(&user_id, KeyUsage::SelfSigning)?,
                user_signing: CrossSigningKeyPair::generate(&user_id, KeyUsage::UserSigning)?,
            },
            backup_key: BackupDecryptionKey::generate(),
        };

        let mut candidates = vec![old.backup_key.clone()];
        candidates.extend(input.unlocked.previous_backup_keys()?);
        candidates.extend(input.local_backup_key.cloned());
        let selection = select_previous_backup_keys(candidates, &input.server.backup_versions);

        let ssss = match input.scope {
            RevocationScope::Full => SsssKey::generate(),
            RevocationScope::DeviceOnly => input.unlocked.key().clone(),
        };
        let current = input
            .server
            .backup_versions
            .iter()
            .max_by_key(|version| version.number)
            .filter(|version| !version.deleted)
            .map(|version| version.version.clone());
        let state = RevocationState {
            version: REVOCATION_STATE_VERSION,
            user_id,
            scope: input.scope,
            stage: RevocationStage::Prepared,
            deleted_devices: input.deleted_devices,
            expected_keys,
            expected_default_key_id: input.unlocked.key().key_id().to_owned(),
            expected_secrets_revision: input.unlocked.secrets_revision(),
            expected_current_version: current,
            key_id: ssss.key_id().to_owned(),
            new_keys: PublicCrossSigningKeys::of(&secrets.cross_signing),
            new_backup_public_key: secrets.backup_key.public_key().to_base64(),
            new_self_signing: base64_encode(*secrets.cross_signing.self_signing.seed()),
            new_user_signing: base64_encode(*secrets.cross_signing.user_signing.seed()),
            new_backup_key: secrets.backup_key.to_base64().to_string(),
            previous_backup_keys: selection
                .keys
                .iter()
                .map(|key| key.to_base64().to_string())
                .collect(),
            versions_without_key: selection.versions_without_key,
            resign,
            secrets_revision: None,
            new_backup_version: None,
        };
        Ok(Self {
            state,
            secrets,
            ssss,
            previous: selection.keys,
        })
    }

    /// 止まった失効を続ける。`unlocked` は開け直した SSSS（2〜5 では新しい SSSS 鍵のスロットで
    /// 開けたもの）。
    ///
    /// - [`RevocationStage::Prepared`] で止まった 2〜5 の失効は続けられない
    ///   （[`CryptoError::RevocationRestart`]。最初からやり直す）。
    /// - SSSS の鍵や秘密が、記録した鍵と違えば [`CryptoError::ServerKeyMismatch`]（別の端末の失効が
    ///   入り混じった。やり直す）。
    pub fn resume(
        state: RevocationState,
        unlocked: &UnlockedSecretStorage,
    ) -> Result<Self, CryptoError> {
        let previous = state.previous_backup_keys()?;
        if state.stage == RevocationStage::Prepared {
            // SSSS を保存する前。端末だけの失効は同じ SSSS 鍵で開け直せば続けられる。
            if state.scope == RevocationScope::Full {
                return Err(CryptoError::RevocationRestart);
            }
            if unlocked.key().key_id() != state.expected_default_key_id {
                return Err(CryptoError::ServerKeyMismatch(
                    "既定の SSSS 鍵が、失効を始めたときと違います".into(),
                ));
            }
            let old = OwnSecrets::from_unlocked(unlocked)?;
            let (self_signing, user_signing, backup_key) = state.new_subkeys()?;
            let secrets = OwnSecrets {
                cross_signing: CrossSigningKeys {
                    master: old.cross_signing.master,
                    self_signing,
                    user_signing,
                },
                backup_key,
            };
            return Ok(Self {
                ssss: unlocked.key().clone(),
                state,
                secrets,
                previous,
            });
        }
        if unlocked.key().key_id() != state.key_id {
            return Err(CryptoError::ServerKeyMismatch(
                "既定の SSSS 鍵が、失効で保存した鍵と違います".into(),
            ));
        }
        let secrets = OwnSecrets::from_unlocked(unlocked)?;
        if PublicCrossSigningKeys::of(&secrets.cross_signing) != state.new_keys
            || secrets.backup_key.public_key().to_base64() != state.new_backup_public_key
        {
            return Err(CryptoError::ServerKeyMismatch(
                "SSSS の鍵が、失効で作った鍵と違います".into(),
            ));
        }
        Ok(Self {
            ssss: unlocked.key().clone(),
            state,
            secrets,
            previous,
        })
    }

    /// 暗号ストアに保存する進み具合。
    pub fn state(&self) -> &RevocationState {
        &self.state
    }

    /// 保存が 1 つ済んだら進める。呼んだあとは [`state`](Self::state) を保存し直す。
    pub fn advance(&mut self, stage: RevocationStage) {
        if stage > self.state.stage {
            self.state.stage = stage;
        }
    }

    /// 作った鍵（新しいマスター鍵を含む）。保存を終えたら [`OwnSecrets::own_keys`] を暗号ストアに置く。
    pub fn secrets(&self) -> &OwnSecrets {
        &self.secrets
    }

    /// SSSS 鍵（2〜5 では新しい鍵）。2〜5 の鍵スロットは、この鍵で作る。
    pub fn ssss_key(&self) -> &SsssKey {
        &self.ssss
    }

    /// SSSS の保存の要求。2〜5 は `POST /api/e2ee/secret_storage/keys`（`slots` は新しい SSSS 鍵の
    /// スロット。1 つ以上）、端末だけは `POST /api/e2ee/secret_storage/secrets`（`slots` は空）。
    /// どちらも同じ要求で `zoo.megolm_backup.previous` に古い版の鍵を退避する。
    pub fn secret_storage_request(&self, slots: &[KeySlot]) -> Result<Value, CryptoError> {
        match self.state.scope {
            RevocationScope::Full => self.ssss.keys_request(&SecretStorageKeysRequest {
                expected_default_key_id: Some(&self.state.expected_default_key_id),
                expected_secrets_revision: self.state.expected_secrets_revision,
                cross_signing: &self.secrets.cross_signing,
                backup_key: &self.secrets.backup_key,
                previous_backup_keys: &self.previous,
                slots,
            }),
            RevocationScope::DeviceOnly => {
                if !slots.is_empty() {
                    return Err(CryptoError::InvalidSecretStorage(
                        "端末だけの失効では鍵スロットを作り直しません".into(),
                    ));
                }
                let keys = &self.secrets.cross_signing;
                let mut secrets: Vec<(&str, EncryptedSecret)> = vec![
                    (
                        SECRET_MEGOLM_BACKUP,
                        self.ssss.encrypt_backup_key(&self.secrets.backup_key),
                    ),
                    self.ssss.encrypt_cross_signing_key(&keys.self_signing),
                    self.ssss.encrypt_cross_signing_key(&keys.user_signing),
                ];
                if !self.previous.is_empty() {
                    secrets.push((
                        SECRET_PREVIOUS_MEGOLM_BACKUP,
                        self.ssss.encrypt_previous_backup_keys(&self.previous),
                    ));
                }
                Ok(self
                    .ssss
                    .secrets_request(self.state.expected_secrets_revision, &secrets))
            }
        }
    }

    /// SSSS を保存した（応答の、または読み直した `secrets_revision`）。
    pub fn secret_storage_saved(&mut self, secrets_revision: Option<u64>) {
        self.state.secrets_revision = secrets_revision;
        self.advance(RevocationStage::SecretStorageSaved);
    }

    /// `POST /api/e2ee/cross_signing/keys` の本文（`expected_keys` は始める前のサーバーの鍵）。
    /// 端末だけの失効ではマスター鍵を送らない。直前の再認証が要る。
    pub fn cross_signing_request(&self) -> Value {
        let upload = self.secrets.cross_signing.upload_json();
        let mut body = json!({
            "expected_keys": {
                "master": self.state.expected_keys.master,
                "self_signing": self.state.expected_keys.self_signing,
                "user_signing": self.state.expected_keys.user_signing,
            },
            "self_signing_key": upload["self_signing_key"],
            "user_signing_key": upload["user_signing_key"],
        });
        if self.state.scope == RevocationScope::Full {
            body["master_key"] = upload["master_key"].clone();
        }
        body
    }

    /// 新しい自己署名鍵による今の端末への署名（`POST /api/e2ee/cross_signing/signatures`）と、
    /// 署名し直すマスター鍵への署名（同じ API。直前の再認証が要る）と、作り直すピン
    /// （`POST /api/e2ee/tofu_pins` の `replace: true`。直前の再認証が要る）。
    ///
    /// 他の自分の端末には署名しない（それぞれが解錠し直して自分に署名する）。
    pub fn signatures(&self, device_keys: &Value) -> Result<RevocationSignatures, CryptoError> {
        let keys = &self.secrets.cross_signing;
        let device = keys.self_signing.sign_device(device_keys)?;
        let (pins, master_keys) = self.state.resign.sign(&keys.user_signing)?;
        Ok(RevocationSignatures {
            device,
            master_keys,
            pins,
        })
    }

    /// `POST /api/e2ee/key_backup/versions` の本文（`expected_current_version` は始める前の今の版）。
    /// 直前の再認証が要る。
    pub fn backup_version_request(&self) -> Result<Value, CryptoError> {
        self.secrets.backup_key.create_version_request(
            &self.secrets.cross_signing.master,
            self.state.expected_current_version.as_deref(),
        )
    }

    /// 新しい版を作った。
    pub fn backup_created(&mut self, version: String) {
        self.state.new_backup_version = Some(version);
        self.advance(RevocationStage::BackupCreated);
    }

    /// 新しい版に移す古い版と、その鍵（`GET /api/e2ee/key_backup/versions` を読み直したもの）。
    /// 新しい版と、消した版は含めない。
    pub fn versions_to_migrate(
        &self,
        versions: &[BackupVersionInfo],
    ) -> Vec<(String, BackupDecryptionKey)> {
        let new_version = self.state.new_backup_version.as_deref();
        versions
            .iter()
            .filter(|v| !v.deleted && Some(v.version.as_str()) != new_version)
            .filter_map(|v| {
                self.previous
                    .iter()
                    .find(|key| key.public_key() == v.public_key)
                    .map(|key| (v.version.clone(), key.clone()))
            })
            .collect()
    }

    /// 古い版の行（`GET /api/e2ee/key_backup/keys?version=<古い版>`）を、古い鍵で復号して新しい版の
    /// 公開鍵で暗号化し直す。返した行を `POST /api/e2ee/key_backup/keys?version=<新しい版>` で送る。
    /// 復号できない行と、行と平文が合わない行は捨て、その数を返す。
    ///
    /// 古い版の `auth_data` の署名は確かめない（docs/e2ee.md の「古い版の秘密鍵の退避」）。送る行の
    /// `is_verified` は `false` にする（取り込んだセッションは送信者を確かめられない）。
    pub fn reencrypt_rows(
        &self,
        old_key: &BackupDecryptionKey,
        rows: &[BackupRow],
    ) -> (Vec<BackupRow>, usize) {
        let target = BackupEncryptionKey::new(self.secrets.backup_key.public_key());
        let mut out = Vec::with_capacity(rows.len());
        let mut dropped = 0;
        for row in rows {
            let reencrypted = old_key
                .decrypt_row(row)
                .and_then(|_| old_key.decrypt(&row.session_data))
                .and_then(|data| target.encrypt(&data));
            match reencrypted {
                Ok(session_data) => out.push(BackupRow {
                    channel_id: row.channel_id.clone(),
                    sender_key: row.sender_key.clone(),
                    session_id: row.session_id.clone(),
                    first_message_index: row.first_message_index,
                    forwarded_count: row.forwarded_count,
                    is_verified: false,
                    session_data,
                }),
                Err(_) => dropped += 1,
            }
        }
        (out, dropped)
    }

    /// 保存を終えたあとに読み直した値を、自分が作った鍵と比べる（docs/e2ee.md の「失効を行う端末は、
    /// 保存を終えたら次のものを読み直し」）。
    ///
    /// - `secret_storage` は `GET /api/e2ee/secret_storage`。
    /// - `keys_query` は自分だけを問い合わせた `keys/query`。
    /// - `current_version` は `GET /api/e2ee/key_backup/versions/current`。
    ///
    /// 一致しなければ [`CryptoError::ServerKeyMismatch`]（別の端末の失効と保存が入り混じった。
    /// SSSS を開け直してやり直す）。
    pub fn verify(
        &self,
        secret_storage: &Value,
        keys_query: &Value,
        current_version: &Value,
    ) -> Result<(), CryptoError> {
        let user_id = self.state.user_id.as_str();
        let mismatch =
            |what: &str| Err(CryptoError::ServerKeyMismatch(format!("{what}が違います")));
        let storage = SecretStorage::from_json(user_id, secret_storage)?
            .ok_or_else(|| CryptoError::ServerKeyMismatch("SSSS がありません".into()))?;
        if storage.default_key_id() != self.ssss.key_id() {
            return mismatch("既定の SSSS 鍵");
        }
        let unlocked = storage.unlock_with_key(&self.ssss)?;
        let stored = OwnSecrets::from_unlocked(&unlocked)?;
        if PublicCrossSigningKeys::of(&stored.cross_signing) != self.state.new_keys
            || stored.backup_key.public_key() != self.secrets.backup_key.public_key()
        {
            return mismatch("SSSS の秘密");
        }
        let server = ServerKeyState::from_json(user_id, keys_query, &json!({"versions": []}))?;
        let keys = &self.secrets.cross_signing;
        for pair in [&keys.master, &keys.self_signing, &keys.user_signing] {
            if server_public_key(&server, user_id, pair.usage())? != Some(pair.public_key()) {
                return mismatch("サーバーのクロス署名鍵");
            }
        }
        let current = BackupVersionInfo::from_json(current_version)?;
        if current.public_key != self.secrets.backup_key.public_key()
            || Some(current.version.as_str()) != self.state.new_backup_version.as_deref()
        {
            return mismatch("鍵バックアップの今の版");
        }
        Ok(())
    }
}

/// [`Revocation::signatures`] の結果。
#[derive(Debug, Clone)]
pub struct RevocationSignatures {
    /// 今の端末への自己署名。
    pub device: SignatureUpload,
    /// 署名し直したマスター鍵への署名。
    pub master_keys: Vec<SignatureUpload>,
    /// 作り直したピン（`replace: true`）。
    pub pins: Vec<Value>,
}

/// サーバーの鍵の公開鍵。形式が壊れていれば、違う鍵として扱う。
fn server_public_key(
    server: &ServerKeyState,
    user_id: &str,
    usage: KeyUsage,
) -> Result<Option<Ed25519PublicKey>, CryptoError> {
    let value = match usage {
        KeyUsage::Master => server.master.as_ref(),
        KeyUsage::SelfSigning => server.self_signing.as_ref(),
        KeyUsage::UserSigning => server.user_signing.as_ref(),
    };
    value
        .map(|value| {
            verify_cross_signing_key(value, user_id, usage).map_err(|e| {
                CryptoError::ServerKeyMismatch(format!(
                    "サーバーの {} を読めません: {e}",
                    usage.as_str()
                ))
            })
        })
        .transpose()
}
