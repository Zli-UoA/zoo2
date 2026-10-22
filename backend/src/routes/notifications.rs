use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, models::Notification,
    routes::response::ApiResponse, state::AppState,
};

#[derive(Debug, Deserialize, Default)]
pub struct ReadAllNotificationsQuery {
    pub channel_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReadAllNotificationsRequest {
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationResponse {
    pub id: String,
    pub user_id: String,
    pub actor_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_type: String,
    pub is_read: bool,
    pub created_at: String,
}

impl From<Notification> for NotificationResponse {
    fn from(n: Notification) -> Self {
        Self {
            id: n.id,
            user_id: n.user_id,
            actor_id: n.actor_id,
            channel_id: n.channel_id,
            message_id: n.message_id,
            notification_type: n.notification_type,
            is_read: n.is_read,
            created_at: n.created_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadAllResponse {
    pub updated_count: usize,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/notifications/{id}/read",
            patch(mark_notification_read),
        )
        .route(
            "/api/notifications/read-all",
            post(mark_all_notifications_read),
        )
}

/// 単一通知の既読化
///
/// [P2] 既に対象が既読の場合、DB への書き込みを行わず txid: None を返却する。
async fn mark_notification_read(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<NotificationResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("notification id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let notification = Notification::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Notification not found: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    // 存在推測防止のため、他人の通知は 404 を返却
    if notification.user_id != auth_user.id {
        return Err(AppError::NotFound(format!("Notification not found: {id}")));
    }

    // 既に既読の場合は no-op として txid: None を返却
    if notification.is_read {
        return Ok(Json(ApiResponse::new(
            NotificationResponse::from(notification),
            None,
        )));
    }

    Notification::update_by_id(id.as_str())
        .is_read(true)
        .exec(&mut tx)
        .await?;

    let mut updated = notification;
    updated.is_read = true;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        NotificationResponse::from(updated),
        txid,
    )))
}

/// 一括既読化
///
/// クエリパラメータ (?channel_id=...) および JSON ボディ ({ "channel_id": "..." }) の双方から
/// 絞り込み対象のチャンネルIDを受け付け可能。
/// [P2] 更新対象が 0 件の場合、txid: None を返却する。
async fn mark_all_notifications_read(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ReadAllNotificationsQuery>,
    payload_opt: Option<Json<ReadAllNotificationsRequest>>,
) -> Result<Json<ApiResponse<ReadAllResponse>>, AppError> {
    let payload = payload_opt.map(|Json(p)| p).unwrap_or_default();
    let channel_id_opt = query.channel_id.or(payload.channel_id);

    if let Some(ref ch_id) = channel_id_opt {
        Uuid::parse_str(ch_id).map_err(|_| {
            AppError::BadRequest(format!("channel_id is not a valid UUID: {ch_id}"))
        })?;
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let sql = if let Some(ref ch_id) = channel_id_opt {
        format!(
            "UPDATE notifications SET is_read = true WHERE user_id = '{}' AND channel_id = '{}' AND is_read = false RETURNING id",
            auth_user.id, ch_id
        )
    } else {
        format!(
            "UPDATE notifications SET is_read = true WHERE user_id = '{}' AND is_read = false RETURNING id",
            auth_user.id
        )
    };

    let rows = toasty::sql::query(&sql).exec(&mut tx).await?;
    let updated_count = rows.len();

    if updated_count == 0 {
        return Ok(Json(ApiResponse::new(
            ReadAllResponse { updated_count: 0 },
            None,
        )));
    }

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ReadAllResponse { updated_count },
        txid,
    )))
}
