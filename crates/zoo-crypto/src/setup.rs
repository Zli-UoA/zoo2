//! 初回設定と解錠のあとに、サーバーへ何を保存するかを決める（#360）。
//!
//! docs/e2ee.md の「初回設定と解錠の流れ」：
//!
//! - 初回設定は、SSSS を保存したあと、クロス署名鍵、端末への署名、バックアップの版の順に保存する。
//!   どの段で止まっても、次に解錠した端末が続きを保存する（初回設定の再開）。
//! - どの段で止まったかは、サーバーの状態だけで決める。サーバーに鍵や版が「ない」ときは続きを
//!   保存し、「違う」ときは失効の途中か改ざんとして扱い、何も保存しない
//!   （[`CryptoError::ServerKeyMismatch`]）。
//!
//! 初回設定も、SSSS を保存したあとは「サーバーに何もない状態からの再開」として同じ関数で扱う。
//! I/O は持たず、送るべき要求の本文を返す。

use serde::Deserialize;
use serde_json::{Value, json};
use vodozemac::{Ed25519PublicKey, base64_encode};
use zeroize::Zeroizing;

use crate::CryptoError;
use crate::backup::{BackupDecryptionKey, BackupVersionInfo, verify_backup_version};
use crate::cross_signing::{
    CrossSigningKeyPair, CrossSigningKeys, KeySignature, KeyUsage, SignatureCheck, SignatureUpload,
    check_signature, verify_cross_signing_key,
};
use crate::device::DeviceAccount;
use crate::secret_storage::{UnlockedSecretStorage, decode_fixed};
use crate::store::{self, StoreKind};

/// SSSS から取り出した（または初回設定で作った）自分の秘密鍵。
///
/// マスター鍵の秘密鍵を含むので、使い終わったら捨てる。暗号ストアには [`OwnSecrets::own_keys`]
/// （マスター鍵の秘密鍵を除いたもの）だけを保存する。
#[derive(Debug)]
pub struct OwnSecrets {
    /// クロス署名鍵 3 つ。
    pub cross_signing: CrossSigningKeys,
    /// 今の版のバックアップ鍵（`m.megolm_backup.v1`）。
    pub backup_key: BackupDecryptionKey,
}

impl OwnSecrets {
    /// 解錠した SSSS から、4 つの秘密を取り出す。どれかがなければ
    /// [`CryptoError::InvalidSecretStorage`]（サーバーは 4 つがそろわない SSSS を受け付けない）。
    pub fn from_unlocked(unlocked: &UnlockedSecretStorage) -> Result<Self, CryptoError> {
        let missing =
            |name: &str| CryptoError::InvalidSecretStorage(format!("SSSS に {name} がありません"));
        let take = |usage: KeyUsage| {
            unlocked
                .cross_signing_key(usage)?
                .ok_or_else(|| missing(usage.secret_name()))
        };
        Ok(Self {
            cross_signing: CrossSigningKeys {
                master: take(KeyUsage::Master)?,
                self_signing: take(KeyUsage::SelfSigning)?,
                user_signing: take(KeyUsage::UserSigning)?,
            },
            backup_key: unlocked
                .backup_key()?
                .ok_or_else(|| missing(crate::SECRET_MEGOLM_BACKUP))?,
        })
    }

    /// 暗号ストアに保存する鍵（マスター鍵は公開鍵だけ）。
    pub fn own_keys(&self) -> OwnKeys {
        let user_id = self.cross_signing.master.user_id();
        let reload = |pair: &CrossSigningKeyPair| {
            CrossSigningKeyPair::from_seed(user_id, pair.usage(), &pair.seed())
                .expect("同じユーザーと用途のシードからは必ず作り直せる")
        };
        OwnKeys {
            master_public_key: self.cross_signing.master.public_key(),
            self_signing: reload(&self.cross_signing.self_signing),
            user_signing: reload(&self.cross_signing.user_signing),
            backup_key: self.backup_key.clone(),
        }
    }
}

/// 解錠した端末が暗号ストアに置く自分の鍵（docs/e2ee.md の「クロス署名」：自己署名鍵、
/// ユーザー署名鍵、バックアップ鍵の秘密鍵は保存し、マスター鍵の秘密鍵は保存しない）。
#[derive(Debug)]
pub struct OwnKeys {
    /// 自分のマスター鍵の公開鍵。
    pub master_public_key: Ed25519PublicKey,
    /// 自己署名鍵。
    pub self_signing: CrossSigningKeyPair,
    /// ユーザー署名鍵。
    pub user_signing: CrossSigningKeyPair,
    /// 今の版のバックアップ鍵。
    pub backup_key: BackupDecryptionKey,
}

#[derive(serde::Serialize, Deserialize)]
struct OwnKeysJson {
    version: u32,
    master_public_key: String,
    self_signing: String,
    user_signing: String,
    backup_key: String,
}

const OWN_KEYS_VERSION: u32 = 1;

impl OwnKeys {
    /// 暗号ストアの封筒（`kind` は `own_keys`）に入れる。
    pub fn pickle(&self, account: &DeviceAccount, storage_key: &[u8; 32]) -> String {
        let json = Zeroizing::new(
            serde_json::to_string(&OwnKeysJson {
                version: OWN_KEYS_VERSION,
                master_public_key: self.master_public_key.to_base64(),
                self_signing: base64_encode(*self.self_signing.seed()),
                user_signing: base64_encode(*self.user_signing.seed()),
                backup_key: self.backup_key.to_base64().to_string(),
            })
            .expect("文字列だけの構造体は必ず JSON にできる"),
        );
        store::seal(storage_key, StoreKind::OwnKeys, account, json.as_bytes())
    }

    /// [`OwnKeys::pickle`] の値を戻す。
    pub fn from_pickle(
        sealed: &str,
        account: &DeviceAccount,
        storage_key: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        let plaintext = store::open(sealed, storage_key, StoreKind::OwnKeys, account)?;
        let invalid = |message: &str| CryptoError::InvalidPickle(message.to_owned());
        let json: OwnKeysJson = serde_json::from_slice(&plaintext)
            .map_err(|_| invalid("自分の鍵の JSON を読めません"))?;
        let json = Zeroizing::new(json);
        if json.version != OWN_KEYS_VERSION {
            return Err(invalid("自分の鍵の版が違います"));
        }
        let user_id = account.user_id();
        let seed = |name: &str, value: &str| {
            decode_fixed::<32>(name, value)
                .map(Zeroizing::new)
                .map_err(|_| invalid("秘密鍵が 32 バイトの base64 ではありません"))
        };
        let backup = seed("backup_key", &json.backup_key)?;
        Ok(Self {
            master_public_key: Ed25519PublicKey::from_base64(&json.master_public_key)?,
            self_signing: CrossSigningKeyPair::from_seed(
                user_id,
                KeyUsage::SelfSigning,
                &*seed("self_signing", &json.self_signing)?,
            )?,
            user_signing: CrossSigningKeyPair::from_seed(
                user_id,
                KeyUsage::UserSigning,
                &*seed("user_signing", &json.user_signing)?,
            )?,
            backup_key: BackupDecryptionKey::from_bytes(&backup),
        })
    }
}

impl zeroize::Zeroize for OwnKeysJson {
    fn zeroize(&mut self) {
        self.self_signing.zeroize();
        self.user_signing.zeroize();
        self.backup_key.zeroize();
    }
}

/// サーバーにある自分の鍵の状態（`keys/query` の自分の分と、`GET /api/e2ee/key_backup/versions`）。
#[derive(Debug, Clone, Default)]
pub struct ServerKeyState {
    /// 自分のマスター鍵（なければ `None`）。
    pub master: Option<Value>,
    /// 自分の自己署名鍵。
    pub self_signing: Option<Value>,
    /// 自分のユーザー署名鍵。
    pub user_signing: Option<Value>,
    /// `e2ee_key_signatures` の行（自分の端末への自己署名を探す）。
    pub signatures: Vec<KeySignature>,
    /// すべての版（消した版を含む）。
    pub backup_versions: Vec<BackupVersionInfo>,
}

impl ServerKeyState {
    /// `keys/query`（自分だけを問い合わせた応答）と `GET /api/e2ee/key_backup/versions` の応答から作る。
    pub fn from_json(user_id: &str, keys: &Value, versions: &Value) -> Result<Self, CryptoError> {
        let invalid = |message: String| CryptoError::InvalidPayload(message);
        let own = |field: &str| {
            keys.get(field)
                .and_then(|map| map.get(user_id))
                .filter(|value| !value.is_null())
                .cloned()
        };
        let signatures = match keys.get("signatures") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => Vec::<KeySignature>::deserialize(value)
                .map_err(|e| invalid(format!("signatures を読めません: {e}")))?,
        };
        let backup_versions = versions
            .get("versions")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("versions がありません".to_owned()))?
            .iter()
            .map(BackupVersionInfo::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            master: own("master_keys"),
            self_signing: own("self_signing_keys"),
            user_signing: own("user_signing_keys"),
            signatures,
            backup_versions,
        })
    }

    fn key(&self, usage: KeyUsage) -> Option<&Value> {
        match usage {
            KeyUsage::Master => self.master.as_ref(),
            KeyUsage::SelfSigning => self.self_signing.as_ref(),
            KeyUsage::UserSigning => self.user_signing.as_ref(),
        }
    }

    /// サーバーの鍵の公開鍵。形式が壊れていれば、違う鍵として扱う。
    fn public_key(
        &self,
        user_id: &str,
        usage: KeyUsage,
    ) -> Result<Option<Ed25519PublicKey>, CryptoError> {
        self.key(usage)
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

    /// 今の版（番号が最も大きい版。消してあれば `None`）。版が 1 つもなければ `Ok(None)`。
    fn current_version(&self) -> Option<&BackupVersionInfo> {
        self.backup_versions
            .iter()
            .max_by_key(|version| version.number)
    }
}

/// 解錠や初回設定のあとに送る要求。順に送る（クロス署名鍵、端末への署名、バックアップの版）。
#[derive(Debug, Clone, Default)]
pub struct ResumePlan {
    /// `POST /api/e2ee/cross_signing/keys` の本文（`expected_keys` 付き）。直前の再認証が要る。
    pub cross_signing: Option<Value>,
    /// `POST /api/e2ee/cross_signing/signatures` に送る、自己署名鍵による今の端末への署名。
    pub device_signature: Option<SignatureUpload>,
    /// `POST /api/e2ee/key_backup/versions` の本文（最初の版。`expected_current_version: null`）。
    pub backup_version: Option<Value>,
    /// 今の版（すでにあれば）。
    pub current_backup_version: Option<String>,
}

impl ResumePlan {
    /// クロス署名鍵の保存（直前の再認証が要る）を含むか。
    pub fn requires_reauth(&self) -> bool {
        self.cross_signing.is_some()
    }

    /// 送るものがないか（初回設定が終わっていて、端末にも署名がある）。
    pub fn is_empty(&self) -> bool {
        self.cross_signing.is_none()
            && self.device_signature.is_none()
            && self.backup_version.is_none()
    }
}

/// 解錠した SSSS の鍵と、サーバーの状態を比べ、続きに送る要求を決める（初回設定の再開と、
/// 新しい端末での解錠の 3、4）。
///
/// - サーバーのクロス署名鍵が SSSS の鍵と違う、版があるのにクロス署名鍵がそろっていない、
///   今の版を消してある、今の版の公開鍵が SSSS のバックアップ鍵と違うときは
///   [`CryptoError::ServerKeyMismatch`] を返し、何も送らせない。
/// - `device_keys` は今の端末の署名付きの端末鍵。サーバーに今の自己署名鍵の有効な署名がなければ
///   署名する。
pub fn plan_resume(
    secrets: &OwnSecrets,
    server: &ServerKeyState,
    device_keys: &Value,
) -> Result<ResumePlan, CryptoError> {
    let keys = &secrets.cross_signing;
    let user_id = keys.master.user_id().to_owned();
    let pairs = [&keys.master, &keys.self_signing, &keys.user_signing];

    let mut server_keys = Vec::with_capacity(3);
    for pair in pairs {
        let on_server = server.public_key(&user_id, pair.usage())?;
        if let Some(public_key) = on_server
            && public_key != pair.public_key()
        {
            return Err(CryptoError::ServerKeyMismatch(format!(
                "サーバーの {} が SSSS の鍵と違います",
                pair.usage().as_str()
            )));
        }
        server_keys.push(on_server);
    }
    let all_present = server_keys.iter().all(Option::is_some);

    let mut plan = ResumePlan::default();
    match server.current_version() {
        None => {
            plan.backup_version = Some(
                secrets
                    .backup_key
                    .create_version_request(&keys.master, None)?,
            );
        }
        Some(current) => {
            // 版が 1 つ以上あれば、初回設定は済んでいる。クロス署名鍵がそろっていないのは
            // 初回設定の再開ではない。
            if !all_present {
                return Err(CryptoError::ServerKeyMismatch(
                    "鍵バックアップの版があるのに、クロス署名鍵がそろっていません".to_owned(),
                ));
            }
            if current.deleted {
                return Err(CryptoError::ServerKeyMismatch(
                    "鍵バックアップの今の版がありません".to_owned(),
                ));
            }
            let version = json!({
                "version": current.version,
                "algorithm": current.algorithm,
                "auth_data": current.auth_data,
            });
            match verify_backup_version(
                &version,
                &user_id,
                &keys.master.public_key(),
                &secrets.backup_key,
            ) {
                Ok(verified) => plan.current_backup_version = Some(verified.version),
                Err(CryptoError::BackupKeyMismatch) => {
                    return Err(CryptoError::ServerKeyMismatch(
                        "鍵バックアップの今の版が SSSS のバックアップ鍵と違います".to_owned(),
                    ));
                }
                Err(e) => {
                    return Err(CryptoError::ServerKeyMismatch(format!(
                        "鍵バックアップの今の版を確かめられません: {e}"
                    )));
                }
            }
        }
    }

    if !all_present {
        plan.cross_signing = Some(cross_signing_request(keys, &server_keys));
    }

    let device_id = device_keys
        .get("device_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CryptoError::InvalidDeviceKeys("device_id がありません".to_owned()))?;
    let signed = plan.cross_signing.is_none()
        && check_signature(
            device_keys,
            &user_id,
            device_id,
            &user_id,
            &keys.self_signing.public_key(),
            &server.signatures,
        ) == SignatureCheck::Valid;
    if !signed {
        plan.device_signature = Some(keys.self_signing.sign_device(device_keys)?);
    }
    Ok(plan)
}

/// E2EE のリセット（SSSS を開けないとき。他人が作った SSSS を含む）で、新しく作った鍵に
/// 差し替える要求を作る。SSSS の作り直し（`POST /api/e2ee/secret_storage/keys`。前提の値は
/// `GET /api/e2ee/secret_storage` で読んだもの）を保存したあとに送る。
///
/// - クロス署名鍵は 3 つとも差し替える（`expected_keys` はサーバーの今の鍵）。
/// - 鍵バックアップは新しい版を作る（`expected_current_version` はサーバーの今の版）。今の版を
///   消してあれば `null`。どちらも直前の再認証が要る。
pub fn plan_reset(
    secrets: &OwnSecrets,
    server: &ServerKeyState,
    device_keys: &Value,
) -> Result<ResumePlan, CryptoError> {
    let keys = &secrets.cross_signing;
    let user_id = keys.master.user_id().to_owned();
    let mut server_keys = Vec::with_capacity(3);
    for usage in [
        KeyUsage::Master,
        KeyUsage::SelfSigning,
        KeyUsage::UserSigning,
    ] {
        // 読めない鍵は、差し替えの前提にできないので、ないものとして扱う（サーバーが 409 を返す）。
        server_keys.push(server.public_key(&user_id, usage).ok().flatten());
    }
    let mut body = cross_signing_request(keys, &[None, None, None]);
    body["expected_keys"] = cross_signing_request(keys, &server_keys)["expected_keys"].clone();
    let current = server
        .current_version()
        .filter(|version| !version.deleted)
        .map(|version| version.version.clone());
    Ok(ResumePlan {
        cross_signing: Some(body),
        device_signature: Some(keys.self_signing.sign_device(device_keys)?),
        backup_version: Some(
            secrets
                .backup_key
                .create_version_request(&keys.master, current.as_deref())?,
        ),
        current_backup_version: None,
    })
}

/// ない鍵だけを送る `POST /api/e2ee/cross_signing/keys` の本文。マスター鍵がなければ 3 つとも送る
/// （サーバーは、送ったマスター鍵の署名がない自己署名鍵とユーザー署名鍵を拒否するため）。
fn cross_signing_request(
    keys: &CrossSigningKeys,
    server_keys: &[Option<Ed25519PublicKey>],
) -> Value {
    let upload = keys.upload_json();
    let expected = |index: usize| {
        server_keys[index]
            .map(|key| Value::String(key.to_base64()))
            .unwrap_or(Value::Null)
    };
    let mut body = json!({
        "expected_keys": {
            "master": expected(0),
            "self_signing": expected(1),
            "user_signing": expected(2),
        }
    });
    let master_missing = server_keys[0].is_none();
    for (index, field) in ["master_key", "self_signing_key", "user_signing_key"]
        .into_iter()
        .enumerate()
    {
        if master_missing || server_keys[index].is_none() {
            body[field] = upload[field].clone();
        }
    }
    body
}

/// 解錠済みの端末が起動したときの確認の結果（docs/e2ee.md の「解錠済みの端末は、起動したときと、
/// Shape でサーバーの自己署名鍵が変わったのを見たときに…」）。
#[derive(Debug, Clone)]
pub enum StartupCheck {
    /// 初回設定は終わっていて、今の端末に今の自己署名鍵の署名がある。
    Ready,
    /// 署名がないので、送る。
    SignDevice(SignatureUpload),
    /// 初回設定が途中で止まっている（クロス署名鍵か版がない）。SSSS を開け直して続ける。
    ResumeSetup,
    /// サーバーの鍵が暗号ストアの鍵と違う（失効のあとか、改ざん）。解錠し直す。
    Unlock,
}

/// 暗号ストアの自分の鍵とサーバーの状態から、起動時にすることを決める。
pub fn startup_check(
    own: &OwnKeys,
    server: &ServerKeyState,
    device_keys: &Value,
) -> Result<StartupCheck, CryptoError> {
    let user_id = own.self_signing.user_id().to_owned();
    let master = server.public_key(&user_id, KeyUsage::Master).ok().flatten();
    let self_signing = server
        .public_key(&user_id, KeyUsage::SelfSigning)
        .ok()
        .flatten();
    let user_signing = server
        .public_key(&user_id, KeyUsage::UserSigning)
        .ok()
        .flatten();
    let differs = |server: Option<Ed25519PublicKey>, own: Ed25519PublicKey| {
        server.is_some_and(|key| key != own)
    };
    if differs(master, own.master_public_key)
        || differs(self_signing, own.self_signing.public_key())
        || differs(user_signing, own.user_signing.public_key())
    {
        return Ok(StartupCheck::Unlock);
    }
    let Some(current) = server.current_version() else {
        return Ok(StartupCheck::ResumeSetup);
    };
    if master.is_none() || self_signing.is_none() || user_signing.is_none() {
        // 版があるのにクロス署名鍵がない。初回設定の再開ではないので、解錠し直して確かめる。
        return Ok(StartupCheck::Unlock);
    }
    if current.deleted || current.public_key != own.backup_key.public_key() {
        return Ok(StartupCheck::Unlock);
    }
    let device_id = device_keys
        .get("device_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CryptoError::InvalidDeviceKeys("device_id がありません".to_owned()))?;
    let signed = check_signature(
        device_keys,
        &user_id,
        device_id,
        &user_id,
        &own.self_signing.public_key(),
        &server.signatures,
    ) == SignatureCheck::Valid;
    if signed {
        Ok(StartupCheck::Ready)
    } else {
        Ok(StartupCheck::SignDevice(
            own.self_signing.sign_device(device_keys)?,
        ))
    }
}
