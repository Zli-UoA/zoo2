//! ログイン後の端末の登録、鍵の補充、to-device の処理（#359。React の `deviceManager.ts` と同じ流れ）。
//!
//! 登録を終えたら、開いた暗号ストアを画面（暗号化の画面。#360）と共有し、解錠済みの端末では
//! 起動時の確認（署名し直しと鍵バックアップの送信）を行う。

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::api::ApiClient;
use crate::electric::{Change, ShapeStream};
use crate::http::{Credentials, HttpError};

use super::setup::{self, SharedStore};
use super::store::{
    self, CryptoStore, PendingDeviceDeletion, ToDeviceRow, clear_pending_device_deletion,
    mark_device_deletion_pending, read_pending_device_deletions, update_pending_device_deletion,
};

/// ログアウトで端末の削除 API を待つ時間。オフラインならすぐ諦め、次の起動で消し直す。
const DELETE_TIMEOUT: Duration = Duration::from_secs(5);

/// 登録できたか分からない端末を、404 が続いたら消えたものとする回数（次の起動ごとに 1 回）。
/// React の `MAX_NOT_FOUND`、Android の `E2eeDeviceManager.MAX_NOT_FOUND` と同じ。
pub const MAX_NOT_FOUND: u32 = 3;

/// 端末名（`display_name`）。本人の端末一覧（#362）にだけ出る。
pub fn display_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    };
    format!("zoo2-tui（{os}）")
}

fn status_of(err: &anyhow::Error) -> Option<reqwest::StatusCode> {
    match err.downcast_ref::<HttpError>() {
        Some(HttpError::Status { status, .. }) => Some(*status),
        _ => None,
    }
}

/// 登録の 409（削除済みの device_id）と、鍵のアップロードの 404 は、端末が別の端末から消された印。
fn device_gone(err: &anyhow::Error) -> bool {
    matches!(
        status_of(err),
        Some(reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::CONFLICT)
    )
}

struct Counts {
    one_time_keys: usize,
    has_unused_fallback_key: bool,
}

fn counts(value: &Value) -> Counts {
    Counts {
        one_time_keys: value["one_time_key_counts"]["signed_curve25519"]
            .as_u64()
            .unwrap_or(0) as usize,
        has_unused_fallback_key: value["unused_fallback_key_types"]
            .as_array()
            .is_some_and(|types| types.iter().any(|t| t == "signed_curve25519")),
    }
}

async fn ensure_registered(api: &ApiClient, store: &Mutex<CryptoStore>) -> Result<Counts> {
    let (registered, device_id, request) = {
        let store = store.lock().await;
        (
            store.is_registered(),
            store.device_id().to_owned(),
            store.registration_request(),
        )
    };
    if registered {
        let value = api
            .e2ee_upload_keys(
                &device_id,
                Value::Object(Default::default()),
                Value::Object(Default::default()),
            )
            .await?;
        return Ok(counts(&value));
    }
    let mut body = request;
    body["display_name"] = Value::String(display_name());
    let value = api.e2ee_register_device(body).await?;
    store.lock().await.mark_registered()?;
    Ok(counts(&value))
}

async fn replenish(api: &ApiClient, store: &Mutex<CryptoStore>, counts: Counts) -> Result<()> {
    let (device_id, upload) = {
        let mut store = store.lock().await;
        let upload =
            store.prepare_key_upload(counts.one_time_keys, counts.has_unused_fallback_key)?;
        (store.device_id().to_owned(), upload)
    };
    let Some(upload) = upload else {
        return Ok(());
    };
    api.e2ee_upload_keys(&device_id, upload.one_time_keys, upload.fallback_keys)
        .await?;
    store.lock().await.mark_keys_published()
}

enum DeleteOutcome {
    Deleted,
    NotFound,
    Failed,
}

/// 端末を消す。失敗したら印を残し、同じユーザーの次の起動で消し直す。
///
/// 404 は、登録できたと分かっている端末なら、消すものがないので済んだものとする。登録できたか
/// 分からない端末（`unconfirmed`）は、あとから届いた登録で端末ができうるので残し、続けて
/// [`MAX_NOT_FOUND`] 回 404 なら済んだものとする。404 以外の失敗では、回数を 0 に戻す。
///
/// `counted` は、この起動で 404 を数えた端末。同じ端末の 404 は起動ごとに 1 回だけ数える
/// （`spawn` のループの試し直しで上限に達して、削除待ちを早く外さないため）。
pub async fn delete_device_or_remember(
    api: &ApiClient,
    root: &Path,
    target: &PendingDeviceDeletion,
    counted: &mut BTreeSet<String>,
) {
    let _ = mark_device_deletion_pending(root, target);
    let outcome =
        match tokio::time::timeout(DELETE_TIMEOUT, api.e2ee_delete_device(&target.device_id)).await
        {
            Ok(Ok(())) => DeleteOutcome::Deleted,
            Ok(Err(err)) if status_of(&err) == Some(reqwest::StatusCode::NOT_FOUND) => {
                DeleteOutcome::NotFound
            }
            _ => DeleteOutcome::Failed,
        };
    let current = read_pending_device_deletions(root)
        .ok()
        .and_then(|all| all.into_iter().find(|entry| entry.same_device(target)))
        .unwrap_or_else(|| target.clone());
    match outcome {
        // 404 が続いていない。数え直す（間に失敗をはさんだ 404 は、続けて受けたものとしない）。
        DeleteOutcome::Failed if current.not_found > 0 => {
            let _ = update_pending_device_deletion(
                root,
                &PendingDeviceDeletion {
                    not_found: 0,
                    ..current
                },
            );
        }
        DeleteOutcome::Failed => {}
        DeleteOutcome::Deleted => {
            let _ = clear_pending_device_deletion(root, &current);
        }
        DeleteOutcome::NotFound if !current.unconfirmed => {
            let _ = clear_pending_device_deletion(root, &current);
        }
        DeleteOutcome::NotFound => {
            if !counted.insert(current.device_id.clone()) {
                return;
            }
            let not_found = current.not_found + 1;
            if not_found >= MAX_NOT_FOUND {
                let _ = clear_pending_device_deletion(root, &current);
            } else {
                let _ = update_pending_device_deletion(
                    root,
                    &PendingDeviceDeletion {
                        not_found,
                        ..current
                    },
                );
            }
        }
    }
}

/// 前のログアウトで消せなかった端末を、同じユーザーで消し直す。一覧を読めなければ何もしない
/// （ファイルはそのまま残す）。`counted` は [`delete_device_or_remember`] と同じ。
pub async fn retry_pending_device_deletion(
    api: &ApiClient,
    root: &Path,
    user_id: &str,
    counted: &mut BTreeSet<String>,
) {
    let Ok(all) = read_pending_device_deletions(root) else {
        return;
    };
    // 同じ端末の記録が重なっていても（前の形式や手で直したファイル）、1 回だけ消し直す。先の記録で
    // 削除を終えたあとに、後の記録で削除待ちに足し直さないため。
    let mut seen = BTreeSet::new();
    for pending in all {
        if pending.user_id == user_id && seen.insert(pending.device_id.clone()) {
            delete_device_or_remember(api, root, &pending, counted).await;
        }
    }
}

/// 端末の管理のタスクが開いている暗号ストア。登録を終えるまでと、作り直している間は `None`。
pub type StoreWatch = watch::Receiver<Option<(String, SharedStore)>>;

/// 暗号ストアを開き、端末を登録して、to-device を処理し続けるタスクを起動する。
///
/// TUI は全画面なので、失敗は表示せずに間隔をあけて試し直す。登録を終えた暗号ストアは、
/// 返した [`StoreWatch`] で画面に渡す（ユーザー ID と組にする）。
pub fn spawn(
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
    api: ApiClient,
    root: std::path::PathBuf,
    user_id: Option<String>,
) -> (JoinHandle<()>, StoreWatch) {
    let (store_tx, store_rx) = watch::channel(None);
    let task = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let mut user_id = user_id;
        let mut state = RunState::default();
        loop {
            if user_id.is_none() {
                user_id = api.me().await.ok().map(|me| me.id);
            }
            let Some(user_id) = user_id.as_deref() else {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(300));
                continue;
            };
            state.connected = false;
            let result = run(
                &http,
                &api_base_url,
                &credentials,
                &api,
                &root,
                user_id,
                &mut state,
                &store_tx,
            )
            .await;
            // 画面が操作の途中で持っている暗号ストアを手放すまで待つ。同じプロセスでも、開いたままの
            // ストアがあるとロックを取れず、開き直せないため。
            if let Some((_, previous)) = store_tx.send_replace(None) {
                wait_until_released(previous).await;
            }
            match result {
                // ほかの zoo2-tui が開いているなど、試し直しても変わらないときは止める。
                Err(err) if err.to_string().contains("ほかの zoo2-tui") => return,
                _ => {}
            }
            let (delay, next) = retry_delay(backoff, state.connected);
            tokio::time::sleep(delay).await;
            backoff = next;
        }
    });
    (task, store_rx)
}

/// 暗号ストアの最後の持ち主が手放すまで待つ（`store` は手放してから待つ）。
///
/// 期限は設けない。画面の操作は、要求ごとの期限（30 秒）で終わり、再認証を待つ間はストアを持たない。
/// 期限で打ち切って開き直すと、同じプロセスが持つロックで失敗し、ほかの zoo2-tui が開いているときと
/// 区別できずに端末の管理が止まってしまう。待ち終えたあとのロックの失敗は、ほかのプロセスのものになる。
pub async fn wait_until_released(store: SharedStore) {
    let weak = Arc::downgrade(&store);
    drop(store);
    while weak.strong_count() > 0 {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 試し直すまでに待つ時間と、その次の間隔を返す。登録と補充を終えて Shape を受け取れていた
/// （`connected`）なら、そのあとの切断は連続した失敗ではないので、最初の 5 秒から数え直す。
/// 続けて失敗したら倍にする（最大 5 分）。
pub fn retry_delay(backoff: Duration, connected: bool) -> (Duration, Duration) {
    const FIRST: Duration = Duration::from_secs(5);
    let delay = if connected { FIRST } else { backoff };
    (delay, (delay * 2).min(Duration::from_secs(300)))
}

/// `spawn` のループが試し直しをまたいで持つ状態。
#[derive(Default)]
struct RunState {
    /// この回で、登録と補充を終えて Shape を受け取れた（[`retry_delay`]）。回ごとに戻す。
    connected: bool,
    /// この起動で 404 を数えた削除待ちの端末（試し直しのたびに数えないため）。
    counted_not_found: BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)]
async fn run(
    http: &reqwest::Client,
    api_base_url: &str,
    credentials: &Credentials,
    api: &ApiClient,
    root: &Path,
    user_id: &str,
    state: &mut RunState,
    store_tx: &watch::Sender<Option<(String, SharedStore)>>,
) -> Result<()> {
    retry_pending_device_deletion(api, root, user_id, &mut state.counted_not_found).await;
    let opened = CryptoStore::open(root, user_id)?;
    if let Some(replaced) = &opened.replaced_device_id {
        let target = PendingDeviceDeletion::new(user_id, replaced, !opened.replaced_registered);
        delete_device_or_remember(api, root, &target, &mut state.counted_not_found).await;
    }
    let mut store = Arc::new(Mutex::new(opened.store));
    let counts = match ensure_registered(api, &store).await {
        Ok(counts) => counts,
        Err(err) if device_gone(&err) => {
            // 別の端末から消された。作り直して、新しい端末として登録する。
            let old = Arc::try_unwrap(store)
                .map_err(|_| anyhow::anyhow!("暗号ストアを閉じられません"))?
                .into_inner();
            old.destroy()?;
            store = Arc::new(Mutex::new(CryptoStore::open(root, user_id)?.store));
            ensure_registered(api, &store).await?
        }
        Err(err) => return Err(err),
    };
    replenish(api, &store, counts).await?;
    store_tx.send_replace(Some((user_id.to_owned(), store.clone())));
    // 解錠済みの端末は、署名を確かめて、送信待ちを鍵バックアップに送る。失敗しても to-device は止めない
    // （次の起動か、次に鍵を受け取ったときに送り直す）。
    let _ = setup::run_startup_check(api, &store, user_id).await;

    let device_id = store.lock().await.device_id().to_owned();
    let url = format!(
        "{}/api/shapes/e2ee_to_device",
        api_base_url.trim_end_matches('/')
    );
    let mut stream = ShapeStream::new(
        http.clone(),
        url,
        vec![("device_id".into(), device_id.clone())],
        credentials.clone(),
    );
    let mut rows: std::collections::BTreeMap<String, ToDeviceRow> = Default::default();
    loop {
        let batch = stream.next_batch().await?;
        // Shape を受け取れた。このあとの切断は、最初の間隔から試し直す。
        state.connected = true;
        if batch.reset {
            rows.clear();
        }
        for change in batch.changes {
            match change {
                Change::Upsert { key, row } => {
                    rows.insert(key, to_device_row(&row));
                }
                Change::Delete { key } => {
                    rows.remove(&key);
                }
            }
        }
        if rows.is_empty() {
            continue;
        }
        let mut pending: Vec<ToDeviceRow> = rows.values().cloned().collect();
        pending.sort_by(|a, b| a.id.len().cmp(&b.id.len()).then_with(|| a.id.cmp(&b.id)));
        let processed = process(api, &store, &device_id, &pending).await?;
        // ack を送れた行は、Shape の Delete が届く前でも外す。初めの Shape が何回かに分かれて
        // 届くと、Delete より先に次の回が来て、同じ行をもう一度処理してしまうため。
        rows.retain(|_, row| !processed.contains(&row.id));
    }
}

/// to-device の Shape の行を読む。Shape は bigint と jsonb の列を文字列で配るので、`content` は
/// JSON として読み直す（読めなければ `null` にして、復号できないメッセージとして捨てる）。
pub fn to_device_row(row: &crate::electric::Row) -> ToDeviceRow {
    let text = |column: &str| match &row.get(column) {
        Some(Value::String(value)) => value.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    ToDeviceRow {
        id: text("id"),
        sender_user_id: text("sender_user_id"),
        r#type: text("type"),
        content: match row.get("content") {
            Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(Value::Null),
            Some(other) => other.clone(),
            None => Value::Null,
        },
    }
}

/// 行を処理して ack を送り、ack を送れた ID を返す。
async fn process(
    api: &ApiClient,
    store: &SharedStore,
    device_id: &str,
    rows: &[ToDeviceRow],
) -> Result<Vec<String>> {
    let mut senders: Vec<String> = rows.iter().map(|row| row.sender_user_id.clone()).collect();
    // 手順 4 で他のユーザーのマスター鍵を判定するには、自分のマスター鍵とユーザー署名鍵が要る（Web と同じ）。
    senders.push(store.lock().await.user_id().to_owned());
    senders.sort();
    senders.dedup();
    let response = api.e2ee_query_keys(&senders).await?;
    let result = store
        .lock()
        .await
        .process_to_device_with_keys(rows, &response)?;
    if !result.processed.is_empty() {
        api.e2ee_ack_to_device(device_id, &result.processed).await?;
        store.lock().await.acknowledged(&result.processed)?;
    }
    if result.room_keys > 0 {
        // 受け取った鍵を鍵バックアップに送る（解錠済みの端末だけ）。失敗しても ack は済んでいるので、
        // 送信待ちに残して次の機会に送る。
        let _ = setup::upload_key_backup(api, store).await;
    }
    if result.new_olm_sessions > 0 {
        let value = api
            .e2ee_upload_keys(
                device_id,
                Value::Object(Default::default()),
                Value::Object(Default::default()),
            )
            .await?;
        replenish(api, store, counts(&value)).await?;
    }
    Ok(result.processed)
}

/// ログアウトの前に見る、暗号ストアの状態。
pub struct LogoutTarget {
    pub store: CryptoStore,
    pub pending_backup: usize,
    /// 保存鍵を失って作り直したときの、前の端末（これも消す）。
    pub replaced_device_id: Option<String>,
    /// 前の端末の登録が済んでいたか。
    pub replaced_registered: bool,
}

/// ログアウトで消す暗号ストアを開く（`root` にあるもの全部）。
pub fn open_for_logout(root: &Path) -> Result<Vec<LogoutTarget>> {
    store::stored_user_ids(root)?
        .into_iter()
        .map(|user_id| {
            let opened = CryptoStore::open_for_logout(root, &user_id)?;
            let pending_backup = opened.store.pending_backup_count();
            Ok(LogoutTarget {
                store: opened.store,
                pending_backup,
                replaced_device_id: opened.replaced_device_id,
                replaced_registered: opened.replaced_registered,
            })
        })
        .collect()
}

/// ログアウト：暗号ストアを消してから、端末の削除 API を呼ぶ（docs/e2ee.md の「ログアウトと端末の削除」）。
///
/// 登録の応答をなくして手元では未登録のままの端末も、サーバーにはあるかもしれないので消す。
/// ただし、登録できたか分からないので、404 でもすぐには消えたものとせず（`unconfirmed`）、
/// 削除待ちに残して次の起動で消し直す。
///
/// `signed_in_user` は、`api` のトークンのユーザー（`/api/users/me`）。暗号ストアが別のユーザーのもの
/// （そのユーザーでログアウトせずに、別のユーザーでログインし直した）か、ユーザーが分からない
/// ときは削除 API を呼ばない。別のユーザーのトークンで消すと、サーバーは 404 を返し、消えたと
/// 取り違えてしまう。削除待ちに残し、そのユーザーの次の起動で消し直す。
pub async fn destroy_for_logout(
    api: &ApiClient,
    root: &Path,
    target: LogoutTarget,
    signed_in_user: Option<&str>,
) -> Result<()> {
    let user_id = target.store.user_id().to_owned();
    // 登録できたと分かっていなければ、404 でもすぐには消えたものとしない。
    let current = PendingDeviceDeletion::new(
        &user_id,
        target.store.device_id(),
        !target.store.is_registered(),
    );
    let replaced = target.replaced_device_id.as_deref().map(|replaced| {
        PendingDeviceDeletion::new(&user_id, replaced, !target.replaced_registered)
    });
    // 消し切る前に印を残す。API の結果を待たずに、先に暗号ストアを消す。記録できなくても
    // （一覧が壊れているなど）、暗号ストアは消す。
    for pending in std::iter::once(&current).chain(replaced.as_ref()) {
        if let Err(err) = mark_device_deletion_pending(root, pending) {
            eprintln!("削除待ちの端末を記録できません: {err:#}");
        }
    }
    target.store.destroy()?;
    if signed_in_user != Some(user_id.as_str()) {
        return Ok(());
    }
    let mut counted = BTreeSet::new();
    delete_device_or_remember(api, root, &current, &mut counted).await;
    if let Some(replaced) = &replaced {
        delete_device_or_remember(api, root, replaced, &mut counted).await;
    }
    Ok(())
}
