//! 端末一覧と失効の手順（docs/e2ee.md の「失効の手順」「失効と検証の実装（#362）」）。
//!
//! React の `revocationController.ts` と `revocationSession.ts`、Android の `E2eeRevocation` と同じ順で
//! 要求を送る。画面（`ui::devices`）は、ここの関数を次の順に呼ぶ。
//!
//! 1. [`open_secret_storage`]：リカバリーキーで SSSS を開け直す（マスター鍵の秘密鍵は暗号ストアにないため）。
//! 2. [`delete_devices`]：選んだ端末を削除する（端末の削除は失効ではない。続けて 3 以降を行う）。
//! 3. [`begin`]：署名し直す一覧を集め、新しい鍵を作って暗号ストアに保存する（サーバーにはまだ保存しない）。
//! 4. 1〜5（[`RevocationScope::Full`]）なら [`RevocationSession::new_recovery_key`] で新しいリカバリーキーを
//!    作って控えてもらう（TUI はパスキーを使えないので、パスキーのスロットは作らない）。
//! 5. 署名し直さない相手と「新しい版に移せないバックアップ」を見せて確かめてもらう。
//! 6. [`run`]：再認証の ID トークンで、SSSS、クロス署名鍵、署名とピン、鍵バックアップの版、古い版の移行と
//!    削除、読み直しの確認の順に最後まで保存する。保存が 1 つ済むたびに暗号ストアに記録する。
//!
//! 止まったら、SSSS を開け直して [`resume`] で続ける。マスター鍵の秘密鍵と SSSS 鍵は
//! [`RevocationSession`]（メモリー）にだけ置き、画面を閉じたら捨てる。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use zeroize::Zeroizing;
use zoo_crypto::{
    BackupRow, BackupVersionInfo, CryptoError, KeySlot, RecoveryKey, ResignNote, Revocation,
    RevocationScope, RevocationStage, RevocationState, SecretStorage, SignatureUpload,
    UnlockedSecretStorage,
};

use super::setup::{BACKUP_UPLOAD_BATCH, SharedStore};
use crate::api::ApiClient;
use crate::http::ApiError;

/// 失効の誤り（Web の `RevocationConflictError` など）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationError {
    /// 直前の再認証が要る（403 `E2EE_REAUTH_REQUIRED`）。保存した段は残るので、再認証してから
    /// [`run`] を呼び直す（同じ鍵で続ける）。
    ReauthRequired,
    /// 別の端末の失効やリセットと入り混じった（409）。[`abandon`] で記録を捨て、SSSS を開け直して
    /// やり直す。
    Conflict(String),
    /// サーバーの鍵が SSSS の鍵と違う（始めるときの不一致、または保存のあとの読み直しの不一致）。
    ServerKeyMismatch(String),
    /// 終わっていない失効があるので、新しい失効を始めない。
    Pending,
    /// 続ける失効がない（終えたか、捨てた）。
    NothingToContinue,
    /// SSSS がまだない。
    NoSecretStorage,
}

impl std::fmt::Display for RevocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReauthRequired => f.write_str("再認証が必要です"),
            Self::Conflict(code) => write!(f, "別の端末の失効と入り混じりました（{code}）"),
            Self::ServerKeyMismatch(message) => write!(f, "サーバーの鍵が違います: {message}"),
            Self::Pending => f.write_str(
                "終わっていない失効があります。先に「失効を続ける」から最後まで保存してください。",
            ),
            Self::NothingToContinue => f.write_str("続ける失効がありません"),
            Self::NoSecretStorage => f.write_str("暗号化はまだ設定されていません"),
        }
    }
}

impl std::error::Error for RevocationError {}

/// 自分の端末一覧の 1 行（Web の `DeviceView`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    pub device_id: String,
    /// 端末名。攻撃者も付けられるので、端末を見分ける根拠にしない（登録日時と最後に使った日時を並べる）。
    pub display_name: Option<String>,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub current: bool,
    /// 今の自己署名鍵の署名があるか（なければ、解錠し直すまで鍵が届かない）。
    pub signed: bool,
}

/// 自分の端末のうち、今の自己署名鍵の署名がない端末（Web の `DevicesSettings`、Android の
/// `unsignedDevices`）。`keys_query` は自分だけの `keys/query`。表示のために署名があるかだけを見る（検証は
/// しない。鍵を配るかどうかは、送信のときに暗号コアが署名を検証して決める）。自己署名鍵がなければ、すべて
/// 未署名。
pub fn unsigned_devices(
    keys_query: &Value,
    user_id: &str,
    device_ids: &[String],
) -> BTreeSet<String> {
    let key_ids: Vec<&str> = keys_query
        .get("self_signing_keys")
        .and_then(|keys| keys.get(user_id))
        .and_then(|key| key.get("keys"))
        .and_then(Value::as_object)
        .map(|keys| {
            keys.keys()
                .map(String::as_str)
                .filter(|id| id.starts_with("ed25519:"))
                .collect()
        })
        .unwrap_or_default();
    if key_ids.is_empty() {
        return device_ids.iter().cloned().collect();
    }
    let devices = keys_query
        .get("device_keys")
        .and_then(|keys| keys.get(user_id));
    let rows: &[Value] = keys_query
        .get("signatures")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let text = |row: &Value, name: &str| row.get(name).and_then(Value::as_str).map(str::to_owned);
    device_ids
        .iter()
        .filter(|device_id| {
            let embedded = devices
                .and_then(|devices| devices.get(device_id.as_str()))
                .and_then(|device| device.get("signatures"))
                .and_then(|signatures| signatures.get(user_id));
            let signed_in_keys = embedded.is_some_and(|embedded| {
                key_ids
                    .iter()
                    .any(|id| embedded.get(*id).is_some_and(Value::is_string))
            });
            let signed_in_rows = rows.iter().any(|row| {
                text(row, "signer_user_id").as_deref() == Some(user_id)
                    && text(row, "target_user_id").as_deref() == Some(user_id)
                    && text(row, "target_key_id").as_deref() == Some(device_id.as_str())
                    && text(row, "signer_key_id")
                        .is_some_and(|signer| key_ids.contains(&signer.as_str()))
            });
            !signed_in_keys && !signed_in_rows
        })
        .cloned()
        .collect()
}

/// 自分の端末の一覧（`current_device_id` は今の端末。なければ `None`）。
pub async fn load_devices(
    api: &ApiClient,
    user_id: &str,
    current_device_id: Option<&str>,
) -> Result<Vec<DeviceView>> {
    let (rows, keys) = tokio::try_join!(api.e2ee_devices(), api.e2ee_query_own_keys(user_id))?;
    let text = |row: &Value, name: &str| row.get(name).and_then(Value::as_str).map(str::to_owned);
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|row| text(row, "device_id"))
        .collect();
    let unsigned = unsigned_devices(&keys, user_id, &ids);
    Ok(rows
        .iter()
        .filter_map(|row| {
            let device_id = text(row, "device_id")?;
            Some(DeviceView {
                signed: !unsigned.contains(&device_id),
                current: Some(device_id.as_str()) == current_device_id,
                display_name: text(row, "display_name"),
                created_at: text(row, "created_at").unwrap_or_default(),
                last_seen_at: text(row, "last_seen_at"),
                device_id,
            })
        })
        .collect())
}

/// 失効の手順の 1：選んだ端末を削除する（端末の削除は失効ではない。続けて [`begin`] 以降を行う）。
pub async fn delete_devices(api: &ApiClient, device_ids: &[String]) -> Result<()> {
    for device_id in device_ids {
        match api.e2ee_delete_device(device_id).await {
            Ok(()) => {}
            // やり直しでは、前の回で削除できた端末も渡す。もうない端末は削除できたものとして扱う。
            Err(error) if ApiError::status_of(&error) == Some(reqwest::StatusCode::NOT_FOUND) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// 開け直した SSSS（メモリーにだけ置く）。
pub struct OpenedSecretStorage(pub UnlockedSecretStorage);

impl std::fmt::Debug for OpenedSecretStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenedSecretStorage(..)")
    }
}

/// 始める（続ける）前に、リカバリーキーで SSSS を開け直す。続きの保存はしない（初回設定の解錠とは違い、
/// 暗号ストアの自分の鍵も変えない）。
pub async fn open_secret_storage(
    api: &ApiClient,
    user_id: &str,
    recovery_key: &str,
) -> Result<OpenedSecretStorage> {
    let storage = api.e2ee_secret_storage().await?;
    let Some(parsed) = SecretStorage::from_json(user_id, &storage)? else {
        return Err(RevocationError::NoSecretStorage.into());
    };
    let recovery = RecoveryKey::parse(recovery_key)?;
    let unlocked = parsed.unlock_with_recovery_key(&recovery)?;
    if let Some(slot) = parsed
        .slots()
        .iter()
        .find(|slot| slot.kind == zoo_crypto::SlotKind::Recovery)
    {
        api.e2ee_mark_key_slot_used(&slot.id).await;
    }
    Ok(OpenedSecretStorage(unlocked))
}

/// 進めている失効と、2〜5 で作る新しい SSSS 鍵の鍵スロット（メモリーにだけ置く）。
pub struct RevocationSession {
    revocation: Revocation,
    slots: Vec<KeySlot>,
}

impl std::fmt::Debug for RevocationSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RevocationSession(..)")
    }
}

impl RevocationSession {
    fn new(revocation: Revocation) -> Self {
        Self {
            revocation,
            slots: Vec::new(),
        }
    }

    /// 進み具合と、失効の画面に出す一覧。
    pub fn state(&self) -> &RevocationState {
        self.revocation.state()
    }

    /// この失効を見分ける値（作った自己署名鍵の公開鍵）。
    pub fn revocation_id(&self) -> String {
        revocation_id(self.state())
    }

    /// 2〜5：新しい SSSS 鍵のリカバリーキーを作る。返したリカバリーキーを利用者に控えてもらう。1〜5 では
    /// 必ず作る（古いリカバリーキーで新しい SSSS 鍵のスロットを作る経路は持たない）。呼び直すと置き換える。
    pub fn new_recovery_key(&mut self, user_id: &str) -> Result<Zeroizing<String>> {
        let recovery = RecoveryKey::generate();
        let slot = KeySlot::new_recovery(user_id, self.revocation.ssss_key(), &recovery)?;
        self.slots
            .retain(|slot| slot.kind != zoo_crypto::SlotKind::Recovery);
        self.slots.push(slot);
        Ok(recovery.to_display())
    }

    /// 新しいリカバリーキーを作ったか（1〜5 は、作るまで実行できない）。
    pub fn has_recovery_slot(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.kind == zoo_crypto::SlotKind::Recovery)
    }
}

/// 失効を見分ける値（作った自己署名鍵の公開鍵。始めるたびに変わる）。
pub fn revocation_id(state: &RevocationState) -> String {
    state.new_keys.self_signing.clone().unwrap_or_default()
}

/// 失効の画面に出す、署名し直さなかった相手と理由（`user_id` の順）。
pub fn not_resigned_users(state: &RevocationState) -> Vec<(String, Vec<ResignNote>)> {
    state
        .resign
        .notes
        .iter()
        .filter(|(_, notes)| !notes.is_empty())
        .map(|(user, notes)| (user.clone(), notes.iter().copied().collect()))
        .collect()
}

/// 失効を始める。署名し直す一覧を集め、新しい鍵を作って暗号ストアに保存する（サーバーにはまだ何も
/// 保存しない）。終わっていない失効があれば [`RevocationError::Pending`]。端末だけの失効で SSSS の鍵と
/// サーバーの鍵が違えば [`RevocationError::ServerKeyMismatch`]（1〜5 を選ぶ）。
///
/// `replacing` はやり直しで捨てる失効の ID（[`Resumed::Restart`] と [`Resumed::Conflict`] の
/// `revocation_id`）。その記録は、新しい失効の記録の保存で置き換える（始め直しに失敗しても、元の記録が残る）。
pub async fn begin(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    scope: RevocationScope,
    deleted_devices: Vec<String>,
    opened: &OpenedSecretStorage,
    replacing: Option<&str>,
) -> Result<RevocationSession> {
    if let Some(state) = store.lock().await.revocation_state()?
        && replacing != Some(revocation_id(&state).as_str())
    {
        return Err(RevocationError::Pending.into());
    }
    let (keys_query, versions, pins) = tokio::try_join!(
        api.e2ee_query_own_keys(user_id),
        api.e2ee_backup_versions(),
        api.e2ee_all_tofu_pins(),
    )?;
    // 署名し直す候補：ピンか記録がある相手（この相手を `keys/query` で問い合わせ、`signatures` と
    // `master_keys` を使う。自分を署名者とする署名を持つ相手は、受け入れでも SAS でもピンを作る）。
    let mut targets: BTreeSet<String> = pins
        .iter()
        .filter_map(|pin| pin.get("target_user_id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    targets.extend(store.lock().await.trust_recorded_users());
    targets.remove(user_id);
    let targets: Vec<String> = targets.into_iter().collect();
    let targets_query = if targets.is_empty() {
        None
    } else {
        Some(api.e2ee_query_keys(&targets).await?)
    };
    let revocation = store
        .lock()
        .await
        .begin_revocation(
            scope,
            &keys_query,
            &versions,
            &pins,
            targets_query.as_ref(),
            deleted_devices,
            &opened.0,
            replacing,
        )
        .map_err(|error| match error.downcast_ref::<CryptoError>() {
            Some(CryptoError::ServerKeyMismatch(message)) => {
                RevocationError::ServerKeyMismatch(message.clone()).into()
            }
            _ => error,
        })?;
    Ok(RevocationSession::new(revocation))
}

/// [`resume`] の結果（Web の `ResumeResult`）。
#[derive(Debug)]
pub enum Resumed {
    /// 記録した段から続ける（[`run`] を呼ぶ）。
    Resumed(Box<RevocationSession>),
    /// 1〜5 を SSSS の保存の前に止めたので、最初からやり直す。範囲と削除した端末は引き継ぐ（範囲を黙って
    /// 端末だけに縮めないため）。記録は、同じ SSSS で `replacing` に `revocation_id` を渡した [`begin`] が
    /// 置き換えるまで残す（始め直しの通信に失敗しても、削除した端末を失わないため）。
    Restart {
        revocation_id: String,
        scope: RevocationScope,
        deleted_devices: Vec<String>,
        opened: OpenedSecretStorage,
    },
    /// 別の端末が SSSS を作り直したので続けられない。記録は残る。「やり直す」では、この失効の記録を
    /// [`abandon`] で捨ててから、同じ範囲と端末で始め直す。
    Conflict {
        revocation_id: String,
        scope: RevocationScope,
        deleted_devices: Vec<String>,
    },
}

/// 止まった失効を、開け直した SSSS で続ける（2〜5 では、SSSS を保存したあとなら新しい SSSS 鍵の
/// リカバリーキーで開ける）。
pub async fn resume(store: &SharedStore, opened: OpenedSecretStorage) -> Result<Resumed> {
    let mut store = store.lock().await;
    let saved = store
        .revocation_state()?
        .ok_or(RevocationError::NothingToContinue)?;
    match store.resume_revocation(&opened.0) {
        Ok(Some(revocation)) => {
            // SSSS を保存したあと、記録する前に止まった 2〜5 は、保存のあとの段から続ける。その段を記録しておく。
            if revocation.state().stage != saved.stage {
                store.save_revocation(&revocation)?;
            }
            Ok(Resumed::Resumed(Box::new(RevocationSession::new(
                revocation,
            ))))
        }
        Ok(None) => Err(RevocationError::NothingToContinue.into()),
        Err(error) => match error.downcast_ref::<CryptoError>() {
            Some(CryptoError::RevocationRestart) => Ok(Resumed::Restart {
                revocation_id: revocation_id(&saved),
                scope: saved.scope,
                deleted_devices: saved.deleted_devices.clone(),
                opened,
            }),
            Some(CryptoError::ServerKeyMismatch(_)) => Ok(Resumed::Conflict {
                revocation_id: revocation_id(&saved),
                scope: saved.scope,
                deleted_devices: saved.deleted_devices.clone(),
            }),
            _ => Err(error),
        },
    }
}

/// やり直すために、保存した失効を捨てる。`expected_id` を渡したら、暗号ストアの記録がその失効のときだけ
/// 捨てる（違えば何もしない）。
pub async fn abandon(store: &SharedStore, expected_id: Option<&str>) -> Result<()> {
    let mut store = store.lock().await;
    if let Some(expected) = expected_id
        && let Some(state) = store.revocation_state()?
        && revocation_id(&state) != expected
    {
        return Err(anyhow!("実行を始めた失効が、別の失効に置き換わりました"));
    }
    store.clear_revocation()
}

/// [`run`] の結果（Web の `RevocationResult`）。
#[derive(Debug, Clone)]
pub struct RevocationOutcome {
    /// 署名し直さなかった相手と理由。
    pub not_resigned: Vec<(String, Vec<ResignNote>)>,
    /// 新しい自己署名鍵で署名されていない自分の端末（解錠し直すまで鍵が届かない。見覚えがなければ削除を
    /// 勧める）。
    pub unsigned_devices: Vec<DeviceView>,
    /// 新しい版に移せなかった行の数。
    pub dropped_sessions: u64,
}

fn signature_json(signature: &SignatureUpload) -> Value {
    json!({
        "target_user_id": signature.target_user_id,
        "target_key_id": signature.target_key_id,
        "signer_key_id": signature.signer_key_id,
        "signature": signature.signature,
    })
}

/// 403 `E2EE_REAUTH_REQUIRED` と 409 を [`RevocationError`] にする。
fn guarded(error: anyhow::Error) -> anyhow::Error {
    if ApiError::has_code(&error, "E2EE_REAUTH_REQUIRED") {
        return RevocationError::ReauthRequired.into();
    }
    if ApiError::status_of(&error) == Some(reqwest::StatusCode::CONFLICT) {
        let code = error
            .downcast_ref::<ApiError>()
            .and_then(|api| api.code.clone())
            .unwrap_or_else(|| "CONFLICT".into());
        return RevocationError::Conflict(code).into();
    }
    error
}

fn secrets_revision_of(storage: &Value) -> Option<u64> {
    storage.get("secrets_revision").and_then(Value::as_u64)
}

/// 新しい版の `upload_key` へのアップロードの証明（docs/e2ee.md の「アップロードの証明」）。フィールドがない版
/// （証明を足す前のサーバー）だけ `None`（証明を付けずに送る）。フィールドがあって文字列でなければ、送らず古い版も
/// 消さずに止める（未証明の行を移したことにしないため）。
async fn upload_proof(
    api: &ApiClient,
    revocation: &Revocation,
    user_id: &str,
    new_version: &str,
) -> Result<Option<String>> {
    let target = api.e2ee_backup_version(new_version).await?;
    match target.as_ref().and_then(|t| t.get("upload_key")) {
        None => Ok(None),
        Some(Value::String(key)) => {
            let key =
                zoo_crypto::Curve25519PublicKey::from_base64(key).map_err(CryptoError::from)?;
            Ok(Some(revocation.secrets().backup_key.upload_proof(
                &key,
                user_id,
                new_version,
            )?))
        }
        Some(_) => Err(anyhow!(
            "版 {new_version} の upload_key が鍵の形ではありません"
        )),
    }
}

/// `GET /api/e2ee/key_backup/keys?version=` の 1 ページ。消した版は `None`。2 つ目は次のページのカーソル
/// （終わりなら `None`）。
async fn backup_rows_page(
    api: &ApiClient,
    version: &str,
    after: Option<&str>,
) -> Result<Option<(Vec<BackupRow>, Option<String>)>> {
    let page = match api.e2ee_backup_keys(version, after).await {
        Ok(page) => page,
        Err(error) if ApiError::status_of(&error) == Some(reqwest::StatusCode::NOT_FOUND) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    // 形の正しくないページを空の版として扱うと、行を移さずに古い版を消してしまう。止めて古い版を残す。
    let Some(Value::Array(sessions)) = page.get("sessions") else {
        return Err(anyhow!("版 {version} の行の一覧が配列ではありません"));
    };
    let rows = sessions
        .iter()
        .map(|row| {
            serde_json::from_value(row.clone())
                .map_err(|e| anyhow!("版 {version} の行を読めません: {e}"))
        })
        .collect::<Result<Vec<BackupRow>>>()?;
    // 終わりは明示した `null` だけ。欠けているか文字列でなければ、残りの行を読まずに古い版を消さないよう止める。
    let next = match page.get("next") {
        Some(Value::String(next)) => Some(next.clone()),
        Some(Value::Null) => None,
        _ => {
            return Err(anyhow!(
                "版 {version} の行の一覧の next が null でも文字列でもありません"
            ));
        }
    };
    Ok(Some((rows, next)))
}

/// 記録した段の続きから、最後まで保存する（SSSS、クロス署名鍵、署名とピン、鍵バックアップの版、古い版の
/// 移行と削除、読み直しの確認）。`reauth` は 5 分以内の再認証の ID トークン（dev モードでは `None`）。
/// `progress` は段が進むたびに呼ぶ。
///
/// - 403 `E2EE_REAUTH_REQUIRED` なら [`RevocationError::ReauthRequired`]（再認証してから呼び直す）。
/// - 409 なら [`RevocationError::Conflict`]、読み直しで一致しなければ [`RevocationError::ServerKeyMismatch`]。
///   どちらも [`abandon`] で記録を捨て、SSSS を開け直してやり直す。
/// - 通信の失敗などはそのまま返す。保存した段は暗号ストアに残るので、呼び直すと続きから送る。
///
/// 暗号ストアは、保存のあいだだけロックを取る（端末の管理のタスクを止めないため）。
pub async fn run(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    session: &mut RevocationSession,
    reauth: Option<&str>,
    progress: &(dyn Fn(RevocationStage) + Sync),
) -> Result<RevocationOutcome> {
    let id = session.revocation_id();
    // 暗号ストアの記録が別の失効に置き換わっていたら（別の画面で捨てて始め直した）、何も保存しない。
    let stored = store.lock().await.revocation_state()?;
    let Some(stored) = stored else {
        return Err(RevocationError::NothingToContinue.into());
    };
    if revocation_id(&stored) != id {
        return Err(anyhow!("実行を始めた失効が、別の失効に置き換わりました"));
    }
    let revocation = &mut session.revocation;
    let stage = |revocation: &Revocation| revocation.state().stage;
    let mut secrets_revision: Option<u64> = None;

    // サーバーへの保存は済んだが、暗号ストアへの記録に失敗した（メモリーの段だけが進んだ）なら、送り直さずに
    // 先に記録し直す。クロス署名鍵の保存のあとは、自分の鍵と記録も新しい鍵にそろえる。
    if stage(revocation) > stored.stage {
        if stage(revocation) >= RevocationStage::CrossSigningSaved
            && stored.stage < RevocationStage::CrossSigningSaved
        {
            let current = api.e2ee_secret_storage().await?;
            if !revocation.secret_storage_matches(&current)? {
                return Err(RevocationError::Conflict("E2EE_SSSS_CONFLICT".into()).into());
            }
            secrets_revision = secrets_revision_of(&current);
            store
                .lock()
                .await
                .apply_revocation(revocation, secrets_revision)?;
        } else {
            store.lock().await.save_revocation(revocation)?;
        }
        progress(stage(revocation));
    }

    if stage(revocation) < RevocationStage::SecretStorageSaved {
        let body = revocation.secret_storage_request(&session.slots)?;
        let result = match revocation.state().scope {
            RevocationScope::Full => api.e2ee_create_secret_storage_key(body, reauth).await,
            RevocationScope::DeviceOnly => api.e2ee_put_secrets(body, reauth).await,
        };
        secrets_revision = match result.map_err(guarded) {
            Ok(revision) => revision,
            Err(error) => {
                // 応答をなくして送り直したか、別の端末が先に書いた。読み直して、自分の書き込みが済んでいれば
                // 続ける（端末だけの失効は鍵 ID が変わらないので、SSSS の秘密を開けて、作った鍵と比べる）。
                if error.downcast_ref::<RevocationError>()
                    != Some(&RevocationError::Conflict("E2EE_SSSS_CONFLICT".into()))
                {
                    return Err(error);
                }
                let current = api.e2ee_secret_storage().await?;
                if !revocation.secret_storage_matches(&current)? {
                    return Err(error);
                }
                secrets_revision_of(&current)
            }
        };
        revocation.secret_storage_saved(secrets_revision);
        store.lock().await.save_revocation(revocation)?;
        progress(stage(revocation));
    }
    // 続きからのときは、この失効で SSSS を保存したときの `secrets_revision`（失効の記録）を使う。今の値を読むと、
    // 別の端末が秘密だけを書き換えたあとの値で移行を続けてしまう。記録にないとき（応答に値がなかった）だけ、
    // SSSS がこの失効の鍵のままかを確かめてから今の値を使う。
    if secrets_revision.is_none() {
        secrets_revision = revocation.state().secrets_revision;
    }
    if secrets_revision.is_none() {
        let current = api.e2ee_secret_storage().await?;
        if !revocation.secret_storage_matches(&current)? {
            return Err(RevocationError::Conflict("E2EE_SSSS_CONFLICT".into()).into());
        }
        secrets_revision = secrets_revision_of(&current);
        // 取り直した値も失効の記録に入れる（このあとの要求の `expected_secrets_revision` は記録から作るので、
        // 記録に入れないと、照合のあとで別の端末が秘密を書き換えても、サーバーが比べずに受け付ける）。
        if secrets_revision.is_some() {
            revocation.secret_storage_saved(secrets_revision);
            store.lock().await.save_revocation(revocation)?;
        }
    }

    if stage(revocation) < RevocationStage::CrossSigningSaved {
        api.e2ee_upload_cross_signing_keys(revocation.cross_signing_request(), reauth)
            .await
            .map_err(guarded)?;
        revocation.advance(RevocationStage::CrossSigningSaved);
        // 記録と自分の鍵を新しい鍵にそろえ、進み具合と同じ書き込みで保存する。
        store
            .lock()
            .await
            .apply_revocation(revocation, secrets_revision)?;
        progress(stage(revocation));
    }

    if stage(revocation) < RevocationStage::SignaturesSaved {
        // 今の端末への自己署名は、相手のマスター鍵への署名と分けて先に送る（相手の都合で止めない）。
        let own = store.lock().await.revocation_signatures(revocation)?;
        api.e2ee_upload_signatures_with_reauth(&[signature_json(&own.device)], reauth)
            .await
            .map_err(guarded)?;
        // 失効を始めてから相手がマスター鍵を作り直していると、その署名はサーバーが 404 で拒む。送る前に今の
        // マスター鍵と照合して外し（注記に回す）、照合と送信のあいだに作り直されたら照合し直す。
        let mut signatures = own;
        let mut attempt = 0;
        loop {
            let targets: Vec<String> = revocation
                .state()
                .resign
                .master_keys
                .iter()
                .map(|m| m.target_user_id.clone())
                .collect();
            if !targets.is_empty() {
                let current = api.e2ee_query_keys(&targets).await?;
                let master_keys: BTreeMap<String, Value> = current
                    .get("master_keys")
                    .and_then(|keys| serde_json::from_value(keys.clone()).ok())
                    .unwrap_or_default();
                if !revocation.drop_stale_master_keys(&master_keys).is_empty() {
                    store.lock().await.save_revocation(revocation)?;
                }
                signatures = store.lock().await.revocation_signatures(revocation)?;
            }
            if signatures.master_keys.is_empty() {
                break;
            }
            let body: Vec<Value> = signatures.master_keys.iter().map(signature_json).collect();
            match api.e2ee_upload_signatures_with_reauth(&body, reauth).await {
                Ok(()) => break,
                Err(error)
                    if ApiError::status_of(&error) == Some(reqwest::StatusCode::NOT_FOUND)
                        && attempt < 2 =>
                {
                    attempt += 1;
                }
                Err(error) => return Err(guarded(error)),
            }
        }
        if !signatures.pins.is_empty() {
            api.e2ee_upload_pins(&signatures.pins, true, reauth)
                .await
                .map_err(guarded)?;
        }
        revocation.advance(RevocationStage::SignaturesSaved);
        store.lock().await.save_revocation(revocation)?;
        progress(stage(revocation));
    }

    if stage(revocation) < RevocationStage::BackupCreated {
        let body = revocation.backup_version_request()?;
        let version = api
            .e2ee_create_backup_version(body, reauth)
            .await
            .map_err(guarded)?
            .ok_or_else(|| anyhow!("版の番号が返りませんでした"))?;
        revocation.backup_created(version);
        store.lock().await.save_revocation(revocation)?;
        progress(stage(revocation));
    }

    if stage(revocation) < RevocationStage::Migrated {
        let new_version = revocation
            .state()
            .new_backup_version
            .clone()
            .ok_or_else(|| anyhow!("新しい版がありません"))?;
        let versions = BackupVersionInfo::list_from_json(&api.e2ee_backup_versions().await?)?;
        let migrate = revocation.versions_to_migrate(&versions);
        // 移行の行も、新しい版の `upload_key` への証明を付けて送る（docs/e2ee.md の「アップロードの証明」）。
        let mut proof: Option<Option<String>> = None;
        // 古い版は 1 ページずつ読み、そのページを新しい版の鍵で暗号化し直して送る（版全体をメモリーに持たない）。
        // 古い版は、`next` が明示的な `null` になり、すべてのページを送れてから消す。
        'versions: for (old_version, old_key) in migrate {
            let mut after: Option<String> = None;
            let mut dropped_total = 0;
            loop {
                // 別の端末が先に移行を終えて消した。
                let Some((rows, next)) =
                    backup_rows_page(api, &old_version, after.as_deref()).await?
                else {
                    continue 'versions;
                };
                let (rows, dropped) = revocation.reencrypt_rows(&old_key, &rows);
                // 捨てた行の数は、失効の記録に版ごとの累計で残す（古い版を消したあとで止まっても数え直せない
                // ため）。止まったあとの再開は最初のページから読み直すので、累計で置き換える。
                dropped_total += dropped;
                revocation.record_dropped(&old_version, dropped_total);
                store.lock().await.save_revocation(revocation)?;
                if !rows.is_empty() {
                    let revision =
                        secrets_revision.ok_or_else(|| anyhow!("secrets_revision がありません"))?;
                    if proof.is_none() {
                        proof = Some(upload_proof(api, revocation, user_id, &new_version).await?);
                    }
                    for chunk in rows.chunks(BACKUP_UPLOAD_BATCH) {
                        api.e2ee_upload_backup_keys(
                            &new_version,
                            revision,
                            json!({ "sessions": chunk }),
                            proof.as_ref().and_then(Option::as_deref),
                        )
                        .await
                        .map_err(guarded)?;
                    }
                }
                match next {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
            // 今の版ではないので、再認証は要らない。
            api.e2ee_delete_backup_version(&old_version, None).await?;
        }
        revocation.advance(RevocationStage::Migrated);
        store.lock().await.save_revocation(revocation)?;
        progress(stage(revocation));
    }

    let (secret_storage, keys_query, current) = tokio::try_join!(
        api.e2ee_secret_storage(),
        api.e2ee_query_own_keys(user_id),
        api.e2ee_current_backup_version(),
    )?;
    // 一覧は、記録を消す前に読む（読めなかったときに、終わった失効を失敗として扱わないため）。
    let devices = load_devices(api, user_id, None).await?;
    revocation
        .verify(
            &secret_storage,
            &keys_query,
            &current.unwrap_or_else(|| json!({})),
        )
        .map_err(|error| RevocationError::ServerKeyMismatch(error.to_string()))?;
    let state = revocation.state();
    let outcome = RevocationOutcome {
        not_resigned: not_resigned_users(state),
        unsigned_devices: devices.into_iter().filter(|d| !d.signed).collect(),
        dropped_sessions: state.dropped_total(),
    };
    store.lock().await.clear_revocation()?;
    progress(RevocationStage::Verified);
    Ok(outcome)
}
