//! ログイン後の端末の登録、鍵の補充、to-device の処理（#359。React の `deviceManager.ts` と同じ流れ）。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::api::ApiClient;
use crate::electric::{Change, ShapeStream};
use crate::http::{Credentials, HttpError};

use super::store::{
    self, CryptoStore, PendingDeviceDeletion, ToDeviceRow, clear_pending_device_deletion,
    mark_device_deletion_pending, read_pending_device_deletion,
};

/// ログアウトで端末の削除 API を待つ時間。オフラインならすぐ諦め、次の起動で消し直す。
const DELETE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// 端末を消す。失敗したら印を残し、同じユーザーの次の起動で消し直す。
pub async fn delete_device_or_remember(
    api: &ApiClient,
    root: &Path,
    user_id: &str,
    device_id: &str,
) {
    let pending = PendingDeviceDeletion {
        user_id: user_id.to_owned(),
        device_id: device_id.to_owned(),
    };
    let _ = mark_device_deletion_pending(root, &pending);
    match tokio::time::timeout(DELETE_TIMEOUT, api.e2ee_delete_device(device_id)).await {
        Ok(Ok(())) => {
            let _ = clear_pending_device_deletion(root);
        }
        // 404 はサーバーに端末がない（登録の前に止まった）。消すものがない。
        Ok(Err(err)) if status_of(&err) == Some(reqwest::StatusCode::NOT_FOUND) => {
            let _ = clear_pending_device_deletion(root);
        }
        _ => {}
    }
}

/// 前のログアウトで消せなかった端末を、同じユーザーで消し直す。
pub async fn retry_pending_device_deletion(api: &ApiClient, root: &Path, user_id: &str) {
    if let Some(pending) = read_pending_device_deletion(root)
        && pending.user_id == user_id
    {
        delete_device_or_remember(api, root, &pending.user_id, &pending.device_id).await;
    }
}

/// 暗号ストアを開き、端末を登録して、to-device を処理し続けるタスクを起動する。
///
/// TUI は全画面なので、失敗は表示せずに間隔をあけて試し直す。
pub fn spawn(
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
    api: ApiClient,
    root: std::path::PathBuf,
    user_id: Option<String>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let mut user_id = user_id;
        loop {
            if user_id.is_none() {
                user_id = api.me().await.ok().map(|me| me.id);
            }
            let Some(user_id) = user_id.as_deref() else {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(300));
                continue;
            };
            match run(&http, &api_base_url, &credentials, &api, &root, user_id).await {
                // ほかの zoo2-tui が開いているなど、試し直しても変わらないときは止める。
                Err(err) if err.to_string().contains("ほかの zoo2-tui") => return,
                _ => {}
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(300));
        }
    })
}

async fn run(
    http: &reqwest::Client,
    api_base_url: &str,
    credentials: &Credentials,
    api: &ApiClient,
    root: &Path,
    user_id: &str,
) -> Result<()> {
    retry_pending_device_deletion(api, root, user_id).await;
    let opened = CryptoStore::open(root, user_id)?;
    if let Some(replaced) = &opened.replaced_device_id {
        delete_device_or_remember(api, root, user_id, replaced).await;
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
        if batch.reset {
            rows.clear();
        }
        for change in batch.changes {
            match change {
                Change::Upsert { key, row } => {
                    let id = match &row["id"] {
                        Value::String(id) => id.clone(),
                        other => other.to_string(),
                    };
                    rows.insert(
                        key,
                        ToDeviceRow {
                            id,
                            sender_user_id: row["sender_user_id"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            r#type: row["type"].as_str().unwrap_or_default().to_owned(),
                            content: row["content"].clone(),
                        },
                    );
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
        process(api, &store, &device_id, &pending).await?;
    }
}

async fn process(
    api: &ApiClient,
    store: &Mutex<CryptoStore>,
    device_id: &str,
    rows: &[ToDeviceRow],
) -> Result<()> {
    let mut senders: Vec<String> = rows.iter().map(|row| row.sender_user_id.clone()).collect();
    senders.sort();
    senders.dedup();
    let response = api.e2ee_query_keys(&senders).await?;
    let result = store
        .lock()
        .await
        .process_to_device(rows, &response["device_keys"])?;
    if !result.processed.is_empty() {
        api.e2ee_ack_to_device(device_id, &result.processed).await?;
        store.lock().await.acknowledged(&result.processed)?;
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
    Ok(())
}

/// ログアウトの前に見る、暗号ストアの状態。
pub struct LogoutTarget {
    pub store: CryptoStore,
    pub pending_backup: usize,
}

/// ログアウトで消す暗号ストアを開く（`root` にあるもの全部）。
pub fn open_for_logout(root: &Path) -> Result<Vec<LogoutTarget>> {
    store::stored_user_ids(root)?
        .into_iter()
        .map(|user_id| {
            let store = CryptoStore::open(root, &user_id)?.store;
            let pending_backup = store.pending_backup_count();
            Ok(LogoutTarget {
                store,
                pending_backup,
            })
        })
        .collect()
}

/// ログアウト：暗号ストアを消してから、端末の削除 API を呼ぶ（docs/e2ee.md の「ログアウトと端末の削除」）。
///
/// 登録の応答をなくして手元では未登録のままの端末も、サーバーにはあるかもしれないので消す
/// （なければ 404 で、消すものがないと分かる）。
pub async fn destroy_for_logout(api: &ApiClient, root: &Path, target: LogoutTarget) -> Result<()> {
    let user_id = target.store.user_id().to_owned();
    let device_id = target.store.device_id().to_owned();
    // 消し切る前に印を残す。API の結果を待たずに、先に暗号ストアを消す。
    mark_device_deletion_pending(
        root,
        &PendingDeviceDeletion {
            user_id: user_id.clone(),
            device_id: device_id.clone(),
        },
    )?;
    target.store.destroy()?;
    delete_device_or_remember(api, root, &user_id, &device_id).await;
    Ok(())
}
