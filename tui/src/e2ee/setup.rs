//! 初回設定と解錠の手順（docs/e2ee.md の「初回設定と解錠の流れ」。#360）。React の
//! `setupController.ts` と同じ流れで、保存の順と冪等の扱いを合わせる。
//!
//! 秘密（マスター鍵の秘密鍵、SSSS 鍵）は [`SetupSession`] にだけ置き、ここではそれが作った要求の
//! 本文をサーバーに送る。画面（`ui::encryption`）はここの関数を順に呼ぶ。

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use zoo_crypto::{CryptoError, ServerKeyState, SetupSession, StartupCheck};

use crate::api::ApiClient;
use crate::http::ApiError;

use super::store::{BackupImportStart, BackupUploadRequest, CryptoStore};

/// 共有する暗号ストア（端末の管理のタスクと画面が使う）。
pub type SharedStore = Arc<Mutex<CryptoStore>>;

/// 1 回に送るセッションの数（サーバーの上限）。
pub const BACKUP_UPLOAD_BATCH: usize = 1000;

/// 今の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E2eeStatus {
    /// SSSS がまだない。初回設定を始める。
    NotSetUp,
    /// SSSS はあるが、この端末は解錠していない。
    Locked(LockReason),
    /// 解錠済み。
    Unlocked,
}

/// 解錠していない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// まだ解錠していない（新しい端末）。
    New,
    /// 解錠済みの端末が、初回設定の途中で止まったのを見つけた。SSSS を開け直して続ける。
    Resume,
    /// 暗号ストアの鍵とサーバーの鍵が違う（失効のあとか、改ざん）。
    Mismatch,
}

/// 初回設定と解錠の誤り（`anyhow::Error` から `downcast_ref` で取り出す）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。同じ鍵のまま、再認証してやり直す。
    ReauthRequired,
    /// 別の端末が先に初回設定を始めた（409 `E2EE_SSSS_CONFLICT`）。手元の鍵を捨てる。
    /// 表示したリカバリーキーは使えない。
    Conflict,
    /// サーバーの鍵が SSSS の鍵と違う（失効が途中か、SSSS が書き換えられた）。
    ServerKeyMismatch(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReauthRequired => write!(f, "再認証が必要です"),
            Self::Conflict => write!(
                f,
                "別の端末が先に初回設定を始めました。表示したリカバリーキーは使えません"
            ),
            Self::ServerKeyMismatch(message) => write!(
                f,
                "失効が終わっていないか、SSSS が書き換えられた可能性があります（{message}）"
            ),
        }
    }
}

impl std::error::Error for SetupError {}

fn is_reauth_required(error: &anyhow::Error) -> bool {
    ApiError::status_of(error) == Some(reqwest::StatusCode::FORBIDDEN)
        && ApiError::has_code(error, "E2EE_REAUTH_REQUIRED")
}

fn is_ssss_conflict(error: &anyhow::Error) -> bool {
    ApiError::status_of(error) == Some(reqwest::StatusCode::CONFLICT)
        && ApiError::has_code(error, "E2EE_SSSS_CONFLICT")
}

/// 403 `E2EE_REAUTH_REQUIRED` を [`SetupError::ReauthRequired`] にする。
fn map_reauth(error: anyhow::Error) -> anyhow::Error {
    if is_reauth_required(&error) {
        SetupError::ReauthRequired.into()
    } else {
        error
    }
}

fn has_secret_storage(storage: &Value) -> bool {
    storage
        .get("default_key_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
}

async fn server_state(api: &ApiClient, user_id: &str) -> Result<ServerKeyState> {
    let (keys, versions) =
        tokio::try_join!(api.e2ee_query_own_keys(user_id), api.e2ee_backup_versions())?;
    Ok(ServerKeyState::from_json(user_id, &keys, &versions)?)
}

/// 今の状態を読む。解錠済みの端末は、起動時の確認（署名し直し、止まった初回設定の発見）も行う。
pub async fn load_status(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
) -> Result<E2eeStatus> {
    let storage = api.e2ee_secret_storage().await?;
    startup_status(api, store, user_id, &storage).await
}

/// `GET /api/e2ee/secret_storage` の応答と暗号ストアから状態を決め、要るなら自分の端末に署名し直す。
///
/// 解錠済みの端末でも、最後に解錠したときの `secrets_revision` がサーバーの値と違えば、SSSS の秘密が
/// 書き換えられた（失効など）ので、暗号ストアの鍵を使わずに解錠し直しを求める（docs/e2ee.md の
/// 「鍵バックアップ」の `secrets_revision` の確認）。古い値のまま鍵バックアップに送り続けないため。
async fn startup_status(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    storage: &Value,
) -> Result<E2eeStatus> {
    if !has_secret_storage(storage) {
        return Ok(E2eeStatus::NotSetUp);
    }
    let recorded = {
        let store = store.lock().await;
        if !store.has_own_keys() {
            return Ok(E2eeStatus::Locked(LockReason::New));
        }
        store.secrets_revision()
    };
    if storage.get("secrets_revision").and_then(Value::as_u64) != recorded {
        return Ok(E2eeStatus::Locked(LockReason::Mismatch));
    }
    let server = server_state(api, user_id).await?;
    let check = store.lock().await.startup_check(&server)?;
    Ok(match check {
        None => E2eeStatus::Locked(LockReason::New),
        Some(StartupCheck::Ready) => E2eeStatus::Unlocked,
        Some(StartupCheck::SignDevice(signature)) => {
            api.e2ee_upload_signatures(json!([signature])).await?;
            E2eeStatus::Unlocked
        }
        Some(StartupCheck::ResumeSetup) => E2eeStatus::Locked(LockReason::Resume),
        Some(StartupCheck::Unlock) => E2eeStatus::Locked(LockReason::Mismatch),
    })
}

/// 起動時の確認（docs/e2ee.md の「初回設定と解錠の流れ」）。解錠済みの端末だけが、自分の端末への
/// 署名を確かめ、なければ署名し直し、送信待ちのセッションを鍵バックアップに送る。まだ解錠していない
/// 端末は、何も取得しない。解錠し直しが要る端末（`secrets_revision` が変わった、鍵が違う）は送らない。
pub async fn run_startup_check(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
) -> Result<E2eeStatus> {
    if !store.lock().await.has_own_keys() {
        return Ok(E2eeStatus::Locked(LockReason::New));
    }
    let storage = api.e2ee_secret_storage().await?;
    let status = startup_status(api, store, user_id, &storage).await?;
    if status == E2eeStatus::Unlocked {
        // 取り込みを入れる前に解錠した端末も、起動したときに取り込む（取り込み終えた版は読み直さない）。
        // 取り込みは裏で行い、止まったら試し直す。
        spawn_backup_import(api.clone(), store);
        upload_key_backup(api, store).await?;
    }
    Ok(status)
}

/// [`upload_key_backup`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupUploadResult {
    Done {
        uploaded: usize,
    },
    /// 今の版を使えない（署名がない、SSSS の鍵と違う、巻き戻りなど）。送っていない。
    Rejected(String),
    /// サーバーが拒んだ（409 `E2EE_WRONG_BACKUP_VERSION` か `E2EE_SSSS_CONFLICT`）。
    Conflict(String),
}

/// 送信を 1 つずつにするロック（呼び出し元が端末の管理でも画面でも、同じストアへの送信を重ねない）。
static BACKUP_UPLOAD: Mutex<()> = Mutex::const_new(());

/// 鍵バックアップの送信待ちのセッションを、今の版に送る（docs/e2ee.md の「鍵バックアップ」）。
/// 解錠していない端末と、今の版がない場合は何もしない。
///
/// 送信は 1 つずつ行う（[`BACKUP_UPLOAD`]）。送っている途中でこの future が捨てられても（画面を閉じて
/// 処理を止めたなど）、次の送信の準備で、その要求の記録を捨てる（セッションは送信待ちのまま残る）。
pub async fn upload_key_backup(api: &ApiClient, store: &SharedStore) -> Result<BackupUploadResult> {
    let _serial = BACKUP_UPLOAD.lock().await;
    if !store.lock().await.has_own_keys() {
        return Ok(BackupUploadResult::Done { uploaded: 0 });
    }
    let current = api.e2ee_current_backup_version().await?;
    let mut uploaded = 0;
    loop {
        let request = store
            .lock()
            .await
            .prepare_backup_upload(current.as_ref(), BACKUP_UPLOAD_BATCH)?;
        let (id, version, secrets_revision, body, count) = match request {
            BackupUploadRequest::None => return Ok(BackupUploadResult::Done { uploaded }),
            BackupUploadRequest::Rejected(reason) => {
                return Ok(BackupUploadResult::Rejected(reason));
            }
            BackupUploadRequest::Upload {
                id,
                version,
                secrets_revision,
                body,
                count,
            } => (id, version, secrets_revision, body, count),
        };
        if let Err(error) = api
            .e2ee_upload_backup_keys(&version, secrets_revision, body)
            .await
        {
            let conflict = ApiError::status_of(&error) == Some(reqwest::StatusCode::CONFLICT)
                && (ApiError::has_code(&error, "E2EE_WRONG_BACKUP_VERSION")
                    || ApiError::has_code(&error, "E2EE_SSSS_CONFLICT"));
            {
                let mut store = store.lock().await;
                store.backup_upload_failed(id, conflict);
                // SSSS の秘密が書き換えられた（失効など）。バックアップ鍵、自己署名鍵、ユーザー署名鍵を使う
                // 処理を止め、解錠し直すまで送らない（docs/e2ee.md の `POST /api/e2ee/key_backup/keys`）。
                if ApiError::has_code(&error, "E2EE_SSSS_CONFLICT") {
                    store.forget_secrets_revision()?;
                }
            }
            if conflict {
                let code = error
                    .downcast_ref::<ApiError>()
                    .and_then(|e| e.code.clone())
                    .unwrap_or_default();
                return Ok(BackupUploadResult::Conflict(code));
            }
            return Err(error);
        }
        store.lock().await.backup_uploaded(id)?;
        uploaded += count;
    }
}

/// 初回設定を保存する。SSSS、クロス署名鍵、端末への署名、バックアップの版の順。
///
/// - 403 `E2EE_REAUTH_REQUIRED` なら [`SetupError::ReauthRequired`]。同じ鍵のまま再認証してやり直す。
/// - SSSS の保存が 409 `E2EE_SSSS_CONFLICT` なら、読み直して、自分の鍵が保存済みなら続ける。
///   別の鍵なら [`SetupError::Conflict`]（手元の鍵を捨てる。表示したリカバリーキーは使えない）。
pub async fn complete_setup(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    session: &SetupSession,
    reauth: Option<&str>,
) -> Result<()> {
    let body = session.secret_storage_request()?;
    let revision = match api.e2ee_create_secret_storage_key(body, reauth).await {
        Ok(revision) => revision,
        Err(error) if is_ssss_conflict(&error) => {
            // 応答をなくして送り直したか、別の端末が先に保存した。読み直して判定する。
            let current = api.e2ee_secret_storage().await?;
            if current.get("default_key_id").and_then(Value::as_str) != Some(session.key_id()) {
                return Err(SetupError::Conflict.into());
            }
            // 前の試行で SSSS を保存したあとに足したスロットは、まだサーバーにない（同じ ID の再送は
            // 何もしない）。
            let saved: Vec<&str> = current
                .get("slots")
                .and_then(Value::as_array)
                .map(|slots| {
                    slots
                        .iter()
                        .filter_map(|slot| slot.get("id").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            for slot in session.pending_slots() {
                if !saved.contains(&slot.id.as_str()) {
                    api.e2ee_add_key_slot(serde_json::to_value(slot)?, reauth)
                        .await
                        .map_err(map_reauth)?;
                }
            }
            current.get("secrets_revision").and_then(Value::as_u64)
        }
        Err(error) => return Err(map_reauth(error)),
    };
    save_remaining(api, store, user_id, session, revision, reauth).await
}

/// SSSS を保存したあと（または解錠したあと）の続きを保存し、自分の鍵を暗号ストアに保存する。
/// 応答をなくして送り直しても、同じ鍵なら成功する（サーバーの冪等の規則）。
///
/// 続きの保存は、作った（開けた）SSSS 鍵がサーバーの既定の鍵のままであることを前提にする
/// （`expected_default_key_id`）。別の端末が SSSS を作り直していたら、何も保存しない。
pub async fn save_remaining(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    session: &SetupSession,
    secrets_revision: Option<u64>,
    reauth: Option<&str>,
) -> Result<()> {
    let plan = plan_remaining(api, store, user_id, session).await?;
    let key_id = session.key_id();
    let with_precondition = |mut body: Value| {
        body["expected_default_key_id"] = Value::String(key_id.to_owned());
        body
    };
    let sent: Result<()> = async {
        if let Some(body) = plan.cross_signing {
            api.e2ee_upload_cross_signing_keys(with_precondition(body), reauth)
                .await?;
        }
        if let Some(signature) = plan.device_signature {
            api.e2ee_upload_signatures(json!([signature])).await?;
        }
        if let Some(body) = plan.backup_version {
            api.e2ee_create_backup_version(with_precondition(body), reauth)
                .await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = sent {
        if is_ssss_conflict(&error) {
            return Err(SetupError::Conflict.into());
        }
        return Err(map_reauth(error));
    }
    store
        .lock()
        .await
        .save_own_keys(session.secrets(), secrets_revision)?;
    // 解錠した（または初回設定を終えた）ので、鍵バックアップの今の版から過去のセッションを取り込み
    // （新しい端末での解錠の手順 5）、手元のセッションを今の版に送る。失敗しても解錠は済んでいる。
    if import_key_backup(api, store).await.is_err() {
        spawn_backup_import(api.clone(), store);
    }
    let _ = upload_key_backup(api, store).await;
    Ok(())
}

/// [`import_key_backup`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupImportResult {
    /// 取り込み終えた（この呼び出しで取り込んだ数）。その版は、次から読み直さない。
    Done { imported: usize },
    /// 取り込むものがない（解錠していない、解錠し直しが要る、今の版がない）。
    None,
    /// 今の版を使えない（署名がない、SSSS の鍵と違う、巻き戻りなど）。取り込まない。
    Rejected(String),
    /// 取り込みの途中で版が消えた（404）。次の解錠か起動で、今の版から取り込む。
    VersionGone,
}

/// 鍵バックアップの今の版から、過去のセッションを取り込む（docs/e2ee.md の「新しい端末での解錠」の
/// 手順 5。Web の `setupController.ts` の `importKeyBackup` と同じ）。
///
/// 今の版をアップロードと同じ `BackupUploadState::accept_current_version` で確かめてから、
/// `GET /api/e2ee/key_backup/keys` を 1 ページずつ読み、ページごとに取り込みと進み具合を保存する。
/// 止まったら（通信の失敗や終了）、同じ版なら続きのページから読む。取り込み終えた版は読み直さない。
/// アップロードと同じ順番待ちで、1 つずつ行う。
pub async fn import_key_backup(api: &ApiClient, store: &SharedStore) -> Result<BackupImportResult> {
    let _serial = BACKUP_UPLOAD.lock().await;
    let current = api.e2ee_current_backup_version().await?;
    let start = store.lock().await.begin_backup_import(current.as_ref())?;
    let (version, mut after) = match start {
        BackupImportStart::Import { version, after } => (version, after),
        BackupImportStart::Done { .. } => return Ok(BackupImportResult::Done { imported: 0 }),
        BackupImportStart::None => return Ok(BackupImportResult::None),
        BackupImportStart::Rejected(message) => return Ok(BackupImportResult::Rejected(message)),
    };
    let mut imported = 0;
    loop {
        let page = match api.e2ee_backup_keys(&version, after.as_deref()).await {
            Ok(page) => page,
            Err(error) if ApiError::status_of(&error) == Some(reqwest::StatusCode::NOT_FOUND) => {
                return Ok(BackupImportResult::VersionGone);
            }
            Err(error) => return Err(error),
        };
        let rows: Vec<zoo_crypto::BackupRow> = page
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| match row {
                // 行が JSON の文字列で届いても読む。
                Value::String(text) => serde_json::from_str(text).ok(),
                other => serde_json::from_value(other.clone()).ok(),
            })
            .collect();
        let next = page
            .get("next")
            .and_then(Value::as_str)
            .filter(|next| after.as_deref() != Some(*next))
            .map(str::to_owned);
        imported += store
            .lock()
            .await
            .import_backup_page(&version, &rows, next.as_deref())?;
        match next {
            Some(next) => after = Some(next),
            None => return Ok(BackupImportResult::Done { imported }),
        }
    }
}

/// 取り込みのやり直しが動いている暗号ストア（`Arc` の指す先）。同じ暗号ストアには重ねて試し直さない。
/// 暗号ストアごとに持つので、古い暗号ストアのやり直しが待っているあいだに開き直した暗号ストアの取り込みも
/// 始められる。やり直しのタスクは `Weak` を持つので、終わるまで同じ番地が別の暗号ストアに使われることは
/// ない。
static BACKUP_IMPORT_RUNNING: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// 鍵バックアップからの取り込みを裏で行い、通信の失敗などで止まったら、30 秒から倍にして最大 10 分の
/// 間隔で、続きのページから試し直す（Web と同じ）。やめるのは、取り込み終えたとき、取り込むものが
/// ない、版を使えない、または途中で版が消えたと分かったとき、暗号ストアが手放されたとき。暗号ストアは、
/// 試すあいだだけ持つ（端末の管理のタスクが開き直せるように）。
pub fn spawn_backup_import(api: ApiClient, store: &SharedStore) {
    let identity = Arc::as_ptr(store) as usize;
    {
        let mut running = BACKUP_IMPORT_RUNNING
            .lock()
            .expect("取り込みのやり直しの記録のロック");
        if running.contains(&identity) {
            return;
        }
        running.push(identity);
    }
    let weak = Arc::downgrade(store);
    tokio::spawn(async move {
        let mut delay = std::time::Duration::from_secs(30);
        loop {
            let Some(store) = weak.upgrade() else {
                break;
            };
            let result = import_key_backup(&api, &store).await;
            drop(store);
            if result.is_ok() {
                break;
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(std::time::Duration::from_secs(600));
        }
        BACKUP_IMPORT_RUNNING
            .lock()
            .expect("取り込みのやり直しの記録のロック")
            .retain(|running| *running != identity);
        // `weak` はここまで持つ（番地を、記録を消すまで別の暗号ストアに使わせない）。
        drop(weak);
    });
}

async fn plan_remaining(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    session: &SetupSession,
) -> Result<zoo_crypto::ResumePlan> {
    let (server, storage) =
        tokio::try_join!(server_state(api, user_id), api.e2ee_secret_storage())?;
    // 別の端末が SSSS を作り直した（同時の初回設定など）。手元の鍵でクロス署名鍵や版を差し替えると、
    // SSSS とサーバーの鍵の世代がずれるので、何も送らない（読んでから送るまでの間の競合は、
    // `expected_default_key_id` でサーバーが防ぐ）。
    if storage.get("default_key_id").and_then(Value::as_str) != Some(session.key_id()) {
        return Err(SetupError::Conflict.into());
    }
    let device_keys = store.lock().await.device_keys();
    match session.plan(&server, &device_keys) {
        Ok(plan) => Ok(plan),
        Err(CryptoError::ServerKeyMismatch(message)) => {
            Err(SetupError::ServerKeyMismatch(message).into())
        }
        Err(error) => Err(error.into()),
    }
}

/// [`unlock`] の結果。
pub struct Unlocked {
    pub session: SetupSession,
    pub secrets_revision: Option<u64>,
    /// 続きの保存（初回設定の再開でクロス署名鍵がない）に再認証が要る。利用者の操作で再認証してから
    /// [`save_remaining`] を呼ぶ。`false` なら最後まで保存してある。
    pub needs_reauth: bool,
}

/// SSSS がまだない（解錠するものがない）。
#[derive(Debug)]
pub struct NoSecretStorage;

impl std::fmt::Display for NoSecretStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "暗号化はまだ設定されていません")
    }
}

impl std::error::Error for NoSecretStorage {}

/// リカバリーキーで SSSS を開ける。続きの保存に再認証が要らなければ、そのまま最後まで保存する。
pub async fn unlock(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    recovery_key: &str,
) -> Result<Unlocked> {
    let storage = api.e2ee_secret_storage().await?;
    let Some(session) = SetupSession::unlock_with_recovery_key(user_id, &storage, recovery_key)?
    else {
        return Err(NoSecretStorage.into());
    };
    if let Some(slot_id) = storage
        .get("slots")
        .and_then(Value::as_array)
        .and_then(|slots| {
            slots
                .iter()
                .find(|slot| slot.get("kind").and_then(Value::as_str) == Some("recovery"))
        })
        .and_then(|slot| slot.get("id").and_then(Value::as_str))
    {
        api.e2ee_mark_key_slot_used(slot_id).await;
    }
    let secrets_revision = session.secrets_revision();
    let plan = plan_remaining(api, store, user_id, &session).await?;
    if plan.requires_reauth() {
        return Ok(Unlocked {
            session,
            secrets_revision,
            needs_reauth: true,
        });
    }
    save_remaining(api, store, user_id, &session, secrets_revision, None).await?;
    Ok(Unlocked {
        session,
        secrets_revision,
        needs_reauth: false,
    })
}
