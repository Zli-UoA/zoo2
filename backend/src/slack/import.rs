//! 管理者による Slack エクスポート ZIP の取り込み API (#295)。
//!
//! 1. `POST /api/admin/slack-imports` でジョブを作り、R2 への presigned PUT を受け取る
//! 2. ブラウザが ZIP を直接 PUT し、`POST .../{id}/uploaded` で解析を依頼する
//! 3. 解析後 (`awaiting_mapping`)、チャンネルの対応付けを `POST .../{id}/start` で送る
//! 4. ワーカー (`slack::worker`) が取り込み、添付ファイルの取得まで進める

use std::{collections::HashSet, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    apps::query,
    auth::{AuthUser, rbac::is_workspace_admin},
    error::AppError,
    state::AppState,
    storage::{delete_storage_object, generate_presigned_put_url, verify_uploaded_object},
    validation::validate_channel_name,
};

pub const ZIP_CONTENT_TYPE: &str = "application/zip";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/slack-imports", get(list).post(create))
        .route("/api/admin/slack-imports/{id}", get(show).delete(cancel))
        .route("/api/admin/slack-imports/{id}/uploaded", post(uploaded))
        .route("/api/admin/slack-imports/{id}/start", post(start))
        .route(
            "/api/admin/slack-imports/{id}/fetch-files",
            post(fetch_files),
        )
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

/// チャンネルごとの取り込み先
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub slack_channel_id: String,
    #[serde(flatten)]
    pub action: MappingAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MappingAction {
    Create {
        name: String,
        #[serde(default)]
        is_private: Option<bool>,
    },
    Existing {
        channel_id: String,
    },
    Skip,
}

const JOB_JSON: &str = "json_build_object('id',j.id,'status',j.status,'file_name',j.file_name,\
'file_size',j.file_size,'team_id',j.team_id,'analysis',j.analysis::jsonb,'mappings',j.mappings::jsonb,\
'total_messages',j.total_messages,'processed_messages',j.processed_messages,\
'total_files',j.total_files,'processed_files',j.processed_files,\
'pending_files',(SELECT count(*) FROM slack_files f WHERE f.job_id=j.id AND f.status='pending'),\
'failed_files',(SELECT count(*) FROM slack_files f WHERE f.job_id=j.id AND f.status='failed'),\
'error',j.error,'created_by',j.created_by,'created_at',j.created_at,'updated_at',j.updated_at,\
'finished_at',j.finished_at)::text";

async fn require_admin(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    user: &AuthUser,
) -> Result<(), AppError> {
    if !is_workspace_admin(tx, &user.id, state.config.dev_auth_enabled).await? {
        return Err(AppError::Forbidden(
            "Only workspace admins can import from Slack".into(),
        ));
    }
    Ok(())
}

impl Mapping {
    /// 取り込み先の zoo チャンネル ID (作成するチャンネルは決定的 ID)
    pub fn target(&self, team: &str) -> Option<String> {
        match &self.action {
            MappingAction::Create { .. } => Some(super::channel_id(team, &self.slack_channel_id)),
            MappingAction::Existing { channel_id } => Some(channel_id.clone()),
            MappingAction::Skip => None,
        }
    }
}

/// 以前の取り込みと別の zoo チャンネルへ入れようとしていないか確かめる。
/// メッセージ ID は取り込み先によらず決まるため、取り込み先を変えると過去ログが移らない。
/// 以前の取り込み先が削除済みなら、過去ログはそこに残ったままなので取り込み直せない。
pub(crate) async fn ensure_same_target(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    mapping: &Mapping,
) -> Result<(), AppError> {
    let Some(target) = mapping.target(team) else {
        return Ok(());
    };
    let previous = query(
        tx,
        "SELECT json_build_object('id',c.id,'name',c.channel_name,'deleted',c.deleted_at IS NOT NULL)::text \
         FROM slack_channels s JOIN channels c ON c.id=s.channel_id \
         WHERE s.team_id=$1 AND s.slack_channel_id=$2",
        &[team, &mapping.slack_channel_id],
    )
    .await?
    .into_iter()
    .next();
    match previous {
        Some(previous) if previous["deleted"] == true => Err(AppError::Conflict(format!(
            "Slack channel {} was imported into #{}, which has been deleted; skip it",
            mapping.slack_channel_id,
            previous["name"].as_str().unwrap_or_default()
        ))),
        Some(previous) if previous["id"] != target.as_str() => Err(AppError::Conflict(format!(
            "Slack channel {} was already imported into #{}; import it into the same channel or skip it",
            mapping.slack_channel_id,
            previous["name"].as_str().unwrap_or_default()
        ))),
        _ => Ok(()),
    }
}

async fn load_job(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<Value, AppError> {
    query(
        tx,
        &format!("SELECT {JOB_JSON} FROM slack_import_jobs j WHERE j.id=$1"),
        &[id],
    )
    .await?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::NotFound("Slack import not found".into()))
}

async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let jobs = query(
        &mut tx,
        &format!("SELECT {JOB_JSON} FROM slack_import_jobs j ORDER BY j.created_at DESC LIMIT 50"),
        &[],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "imports": jobs })))
}

async fn show(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(job))
}

#[derive(Deserialize)]
struct CreateRequest {
    file_name: String,
    file_size: i64,
}

async fn create(
    State(s): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateRequest>,
) -> Result<Json<Value>, AppError> {
    let file_name = body.file_name.trim();
    if file_name.is_empty()
        || file_name.chars().count() > 255
        || file_name.chars().any(char::is_control)
    {
        return Err(AppError::BadRequest("Invalid file name".into()));
    }
    let max = s.config.slack.import_max_zip_size;
    if body.file_size <= 0 || body.file_size as u64 > max {
        return Err(AppError::BadRequest(format!(
            "file_size must be between 1 and {max} bytes"
        )));
    }

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("slack-imports/{id}/export.zip");
    toasty::sql::statement(
        "INSERT INTO slack_import_jobs(id,created_by,file_name,file_size,storage_key) VALUES($1,$2,$3,$4::bigint,$5)",
    )
    .bind(id.as_str())
    .bind(user.id.as_str())
    .bind(file_name)
    .bind(body.file_size.to_string())
    .bind(key.as_str())
    .exec(&mut tx)
    .await?;

    let upload_url = generate_presigned_put_url(
        &s.s3_client,
        &s.config.s3.bucket,
        &key,
        body.file_size,
        ZIP_CONTENT_TYPE,
        Duration::from_secs(s.config.s3.presigned_put_expiry_secs),
        s.config.s3.public_endpoint.as_deref(),
    )
    .await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(json!({
        "import": job,
        "upload_url": upload_url,
        "upload_headers": { "content-type": ZIP_CONTENT_TYPE },
    })))
}

async fn uploaded(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let job = load_job(&mut tx, &id).await?;
    if job["status"] != "uploading" {
        return Err(AppError::Conflict(
            "The export has already been uploaded".into(),
        ));
    }
    let key = format!("slack-imports/{id}/export.zip");
    verify_uploaded_object(
        &s.s3_client,
        &s.config.s3.bucket,
        &key,
        job["file_size"].as_i64().unwrap_or(-1),
        ZIP_CONTENT_TYPE,
    )
    .await?;
    toasty::sql::statement(
        "UPDATE slack_import_jobs SET status='analyzing',available_at=now(),attempts=0,error=NULL,updated_at=now() WHERE id=$1 AND status='uploading'",
    )
    .bind(id.as_str())
    .exec(&mut tx)
    .await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(job))
}

#[derive(Deserialize)]
struct StartRequest {
    mappings: Vec<Mapping>,
}

async fn start(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<StartRequest>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let job = load_job(&mut tx, &id).await?;
    if job["status"] != "awaiting_mapping" {
        return Err(AppError::Conflict(
            "The export is not waiting for channel mappings".into(),
        ));
    }
    let team = job["team_id"].as_str().unwrap_or_default().to_string();
    let known: HashSet<&str> = job["analysis"]["channels"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|c| c["slack_channel_id"].as_str())
                .collect()
        })
        .unwrap_or_default();

    let mut seen_channels = HashSet::new();
    let mut new_names = HashSet::new();
    let mut targets = HashSet::new();
    for mapping in &body.mappings {
        if !known.contains(mapping.slack_channel_id.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Unknown Slack channel {}",
                mapping.slack_channel_id
            )));
        }
        if !seen_channels.insert(mapping.slack_channel_id.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Slack channel {} is mapped twice",
                mapping.slack_channel_id
            )));
        }
        ensure_same_target(&mut tx, &team, mapping).await?;
        match &mapping.action {
            MappingAction::Create { name, .. } => {
                validate_channel_name(name)?;
                if !new_names.insert(name.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "Channel name '{name}' is used twice"
                    )));
                }
                // 同じ Slack チャンネルの再取り込みで作ったチャンネルなら衝突とみなさない
                let own = super::channel_id(&team, &mapping.slack_channel_id);
                let taken = query(
                    &mut tx,
                    "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1 AND id<>$2",
                    &[name, &own],
                )
                .await?;
                if !taken.is_empty() {
                    return Err(AppError::Conflict(format!(
                        "Channel name '{name}' is already taken"
                    )));
                }
            }
            MappingAction::Existing { channel_id } => {
                let found = query(
                    &mut tx,
                    "SELECT json_build_object('id',id)::text FROM channels WHERE id=$1 AND NOT is_dm AND deleted_at IS NULL",
                    &[channel_id],
                )
                .await?;
                if found.is_empty() {
                    return Err(AppError::BadRequest(format!(
                        "Channel {channel_id} does not exist"
                    )));
                }
                if !targets.insert(channel_id.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "Channel {channel_id} is a target of two Slack channels"
                    )));
                }
            }
            MappingAction::Skip => {}
        }
    }
    if !body
        .mappings
        .iter()
        .any(|m| !matches!(m.action, MappingAction::Skip))
    {
        return Err(AppError::BadRequest("Select at least one channel".into()));
    }

    let mappings =
        serde_json::to_string(&body.mappings).map_err(|e| AppError::Internal(e.into()))?;
    toasty::sql::statement(
        "UPDATE slack_import_jobs SET status='importing',mappings=$2,available_at=now(),attempts=0,error=NULL,processed_messages=0,updated_at=now() WHERE id=$1 AND status='awaiting_mapping'",
    )
    .bind(id.as_str())
    .bind(mappings)
    .exec(&mut tx)
    .await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(job))
}

/// トークン設定後に、未取得・取得に失敗した添付ファイルの取得をやり直す
async fn fetch_files(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    if s.config.slack.import_bot_token.is_none() {
        return Err(AppError::BadRequest(
            "SLACK_IMPORT_BOT_TOKEN is not configured".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let job = load_job(&mut tx, &id).await?;
    let retryable =
        job["pending_files"].as_i64().unwrap_or(0) + job["failed_files"].as_i64().unwrap_or(0);
    if job["status"] != "completed" || retryable == 0 {
        return Err(AppError::Conflict(
            "There are no pending files to fetch".into(),
        ));
    }
    // トークンの権限を直した後などに備え、失敗した添付も試行回数を戻して再取得する
    toasty::sql::statement(
        "UPDATE slack_files SET status='pending',attempts=0,error=NULL,updated_at=now() WHERE job_id=$1 AND status='failed'",
    )
    .bind(id.as_str())
    .exec(&mut tx)
    .await?;
    toasty::sql::statement(
        "UPDATE slack_import_jobs SET status='fetching_files',available_at=now(),attempts=0,error=NULL,finished_at=NULL,updated_at=now() WHERE id=$1 AND status='completed'",
    )
    .bind(id.as_str())
    .exec(&mut tx)
    .await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(job))
}

/// 取り込み前のジョブを取り消し、アップロード済みの ZIP を消す
async fn cancel(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &s, &user).await?;
    let job = load_job(&mut tx, &id).await?;
    let status = job["status"].as_str().unwrap_or_default();
    if !matches!(status, "uploading" | "awaiting_mapping" | "failed") {
        return Err(AppError::Conflict(format!(
            "An import in status '{status}' cannot be cancelled"
        )));
    }
    toasty::sql::statement(
        "UPDATE slack_import_jobs SET status='cancelled',lease_until=NULL,lease_token=NULL,finished_at=now(),updated_at=now() WHERE id=$1",
    )
    .bind(id.as_str())
    .exec(&mut tx)
    .await?;
    let job = load_job(&mut tx, &id).await?;
    tx.commit().await?;
    let key = format!("slack-imports/{id}/export.zip");
    if let Err(e) = delete_storage_object(&s.s3_client, &s.config.s3.bucket, &key).await {
        tracing::warn!(error = %e, import_id = %id, "Failed to delete cancelled Slack export");
    }
    Ok(Json(job))
}
