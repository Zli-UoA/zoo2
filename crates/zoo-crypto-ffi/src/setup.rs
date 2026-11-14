//! 初回設定と解錠（#360）。コアの `setup.rs`（[`core::SetupSession`]、[`core::plan_resume`]、
//! [`core::startup_check`]、[`core::OwnKeys`]）を Android から使う。
//!
//! マスター鍵の秘密鍵と SSSS 鍵は [`SetupSession`] の中（Rust のメモリー）にだけ置き、Kotlin には
//! 返さない。Kotlin に返すのは、サーバーに送る要求の本文（JSON）と、利用者に見せるリカバリーキー
//! だけ。暗号ストアには [`SetupSession::own_keys_pickle`]（マスター鍵の秘密鍵を除いた封筒）を置く。

use std::sync::{Arc, Mutex};

use serde_json::Value;
use zoo_crypto as core;

use crate::{
    BackupDecryptionKey, DeviceAccount, Result, TrustStore, UnlockedSecretStorage, bytes32, lock,
    parse_json,
};

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("コアの型は必ず JSON にできる")
}

fn server_state(user_id: &str, keys_query: &str, versions: &str) -> Result<core::ServerKeyState> {
    let keys_query = parse_json("keys_query", keys_query)?;
    let versions = parse_json("versions", versions)?;
    Ok(core::ServerKeyState::from_json(
        user_id,
        &keys_query,
        &versions,
    )?)
}

/// 解錠や初回設定のあとに送る要求（[`core::ResumePlan`]）。順に送る。どれも JSON 文字列。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResumePlan {
    /// `POST /api/e2ee/cross_signing/keys` の本文（`expected_keys` 付き）。直前の再認証が要る。
    pub cross_signing: Option<String>,
    /// `POST /api/e2ee/cross_signing/signatures` の `signatures` の 1 件（今の端末への自己署名）。
    pub device_signature: Option<String>,
    /// `POST /api/e2ee/key_backup/versions` の本文（最初の版）。
    pub backup_version: Option<String>,
    /// 今の版（すでにあれば）。
    pub current_backup_version: Option<String>,
}

impl From<core::ResumePlan> for ResumePlan {
    fn from(plan: core::ResumePlan) -> Self {
        Self {
            cross_signing: plan.cross_signing.as_ref().map(Value::to_string),
            device_signature: plan.device_signature.as_ref().map(to_json),
            backup_version: plan.backup_version.as_ref().map(Value::to_string),
            current_backup_version: plan.current_backup_version,
        }
    }
}

/// 解錠済みの端末が起動したときにすること（[`core::StartupCheck`]）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StartupCheck {
    /// 初回設定は終わっていて、今の端末に今の自己署名鍵の署名がある。
    Ready,
    /// 署名がないので送る（`signature` は `POST /api/e2ee/cross_signing/signatures` の 1 件の JSON）。
    SignDevice { signature: String },
    /// 初回設定が途中で止まっている。SSSS を開け直して続ける。
    ResumeSetup,
    /// サーバーの鍵が暗号ストアの鍵と違う。解錠し直す。
    Unlock,
}

/// 初回設定を始めた結果。`recovery_key` は利用者に見せて控えてもらう表記（保存しない）。
#[derive(uniffi::Record)]
pub struct SetupStart {
    pub session: Arc<SetupSession>,
    pub recovery_key: String,
}

impl std::fmt::Debug for SetupStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // リカバリーキーは出さない。
        f.debug_struct("SetupStart").finish_non_exhaustive()
    }
}

/// 初回設定と解錠のあいだ、メモリーにだけ置く秘密（[`core::SetupSession`]。Web の `setupSession.ts`）。
///
/// 使い終わったら Kotlin で `close()` する（秘密は Rust の側で消える）。
#[derive(uniffi::Object)]
pub struct SetupSession {
    inner: Mutex<core::SetupSession>,
    user_id: String,
}

impl SetupSession {
    fn wrap(inner: core::SetupSession) -> Arc<Self> {
        let user_id = inner.secrets().cross_signing.master.user_id().to_owned();
        Arc::new(Self {
            inner: Mutex::new(inner),
            user_id,
        })
    }
}

#[uniffi::export]
impl SetupSession {
    /// 開けた SSSS から作る（パスキーで開けたとき。`SecretStorage.unlock_with_passkey` の結果）。
    #[uniffi::constructor]
    pub fn from_unlocked(unlocked: &UnlockedSecretStorage) -> Result<Arc<Self>> {
        Ok(Self::wrap(core::SetupSession::from_unlocked(
            &unlocked.inner,
        )?))
    }

    /// SSSS 鍵の ID（`expected_default_key_id` に入れる）。
    pub fn key_id(&self) -> String {
        lock(&self.inner).key_id().to_owned()
    }

    pub fn user_id(&self) -> String {
        self.user_id.clone()
    }

    /// 初回設定で作った鍵か（解錠で取り出した鍵ではない）。
    pub fn is_created(&self) -> bool {
        lock(&self.inner).is_created()
    }

    /// 解錠したときの `secrets_revision`（初回設定では `null`）。
    pub fn secrets_revision(&self) -> Option<u64> {
        lock(&self.inner).secrets_revision()
    }

    /// 初回設定にパスキーのスロットを足す（同じ credential ID のスロットは置き換える）。
    pub fn add_passkey_slot(
        &self,
        credential_id: String,
        prf_salt: Vec<u8>,
        prf_output: Vec<u8>,
        label: Option<String>,
    ) -> Result<()> {
        let prf_salt = bytes32("prf_salt", &prf_salt)?;
        let prf_output = zeroize::Zeroizing::new(bytes32("prf_output", &prf_output)?);
        lock(&self.inner).add_passkey_slot(&credential_id, &prf_salt, &prf_output, label)?;
        Ok(())
    }

    /// 初回設定で作ったスロット（JSON の一覧。保存の再試行で、サーバーにないものを足す）。
    pub fn pending_slots(&self) -> Vec<String> {
        lock(&self.inner)
            .pending_slots()
            .iter()
            .map(to_json)
            .collect()
    }

    /// 初回設定の `POST /api/e2ee/secret_storage/keys` の本文（JSON）。
    pub fn secret_storage_request(&self) -> Result<String> {
        Ok(lock(&self.inner).secret_storage_request()?.to_string())
    }

    /// サーバーの状態（自分だけの `keys/query` と `GET /api/e2ee/key_backup/versions` の応答）と比べ、
    /// 続きに送る要求を決める。鍵が違えば `ServerKeyMismatch`。
    pub fn plan(
        &self,
        keys_query: String,
        versions: String,
        device_keys: String,
    ) -> Result<ResumePlan> {
        let server = server_state(&self.user_id, &keys_query, &versions)?;
        let device_keys = parse_json("device_keys", &device_keys)?;
        Ok(lock(&self.inner).plan(&server, &device_keys)?.into())
    }

    /// 暗号ストアに置く自分の鍵（`OwnKeys::pickle`。マスター鍵の秘密鍵は入れない）。
    pub fn own_keys_pickle(&self, account: &DeviceAccount, storage_key: Vec<u8>) -> Result<String> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        let own = lock(&self.inner).secrets().own_keys();
        Ok(own.pickle(&lock(&account.inner), &storage_key))
    }

    /// 配布先の判定の記録に、自分のマスター鍵とユーザー署名鍵を記録する（TUI の `save_own_keys` と同じ）。
    pub fn record_own_keys(&self, trust: &TrustStore) -> Result<()> {
        let session = lock(&self.inner);
        let keys = &session.secrets().cross_signing;
        let mut trust = lock(&trust.inner);
        trust.trust_own_master_key(&keys.master)?;
        trust.record_own_user_signing_key(&keys.user_signing)?;
        Ok(())
    }
}

/// 初回設定を始める。クロス署名鍵、バックアップ鍵、SSSS 鍵、リカバリーキーを作り、リカバリーキーの
/// スロットを作る（UniFFI のコンストラクターは自分しか返せないので関数にする）。
#[uniffi::export]
pub fn begin_setup(user_id: String) -> Result<SetupStart> {
    let (session, recovery_key) = core::SetupSession::begin(&user_id)?;
    Ok(SetupStart {
        session: SetupSession::wrap(session),
        recovery_key: recovery_key.to_string(),
    })
}

/// `GET /api/e2ee/secret_storage` の応答（JSON）を、利用者が打ち込んだリカバリーキーで開ける。
/// SSSS がまだなければ `null`。打ち間違いは `InvalidRecoveryKey`、鍵が違えば `KeySlotUnlock`。
#[uniffi::export]
pub fn unlock_setup_with_recovery_key(
    user_id: String,
    response: String,
    recovery_key: String,
) -> Result<Option<Arc<SetupSession>>> {
    let response = parse_json("response", &response)?;
    let recovery_key = zeroize::Zeroizing::new(recovery_key);
    Ok(
        core::SetupSession::unlock_with_recovery_key(&user_id, &response, &recovery_key)?
            .map(SetupSession::wrap),
    )
}

/// 暗号ストアに置いた自分の鍵（[`core::OwnKeys`]）。
#[derive(uniffi::Object)]
pub struct OwnKeys {
    inner: core::OwnKeys,
}

#[uniffi::export]
impl OwnKeys {
    /// [`SetupSession::own_keys_pickle`] の値を戻す。
    #[uniffi::constructor]
    pub fn from_pickle(
        sealed: String,
        account: &DeviceAccount,
        storage_key: Vec<u8>,
    ) -> Result<Arc<Self>> {
        let storage_key = zeroize::Zeroizing::new(bytes32("storage_key", &storage_key)?);
        Ok(Arc::new(Self {
            inner: core::OwnKeys::from_pickle(&sealed, &lock(&account.inner), &storage_key)?,
        }))
    }

    /// 自分のマスター鍵の公開鍵（`BackupUploadState.accept_current_version` に渡す）。
    pub fn master_public_key(&self) -> String {
        self.inner.master_public_key.to_base64()
    }

    /// 自分の自己署名鍵の公開鍵（受信時の検証の手順 4 で、自分の別の端末を検証済みにする `SenderTrust` に使う）。
    pub fn self_signing_public_key(&self) -> String {
        self.inner.self_signing.public_key().to_base64()
    }

    /// 今の版のバックアップ鍵。
    pub fn backup_key(&self) -> Arc<BackupDecryptionKey> {
        BackupDecryptionKey::wrap(self.inner.backup_key.clone())
    }

    /// 起動時にすることを決める。
    pub fn startup_check(
        &self,
        keys_query: String,
        versions: String,
        device_keys: String,
    ) -> Result<StartupCheck> {
        let user_id = self.inner.self_signing.user_id().to_owned();
        let server = server_state(&user_id, &keys_query, &versions)?;
        let device_keys = parse_json("device_keys", &device_keys)?;
        Ok(
            match core::startup_check(&self.inner, &server, &device_keys)? {
                core::StartupCheck::Ready => StartupCheck::Ready,
                core::StartupCheck::SignDevice(signature) => StartupCheck::SignDevice {
                    signature: to_json(&signature),
                },
                core::StartupCheck::ResumeSetup => StartupCheck::ResumeSetup,
                core::StartupCheck::Unlock => StartupCheck::Unlock,
            },
        )
    }
}
