use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message, MessageReaction, Notification},
    routes::response::ApiResponse,
    state::AppState,
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
        .route("/api/notifications/{id}/context", get(notification_context))
        .route(
            "/api/notifications/{id}/read",
            patch(mark_notification_read),
        )
        .route(
            "/api/notifications/read-all",
            post(mark_all_notifications_read),
        )
}

#[derive(Serialize)]
struct MessagePreview {
    id: String,
    user_id: String,
    content: String,
}

#[derive(Serialize)]
struct PreviewReaction {
    user_id: String,
    emoji: String,
}

#[derive(Serialize)]
struct NotificationContext {
    message: Option<MessagePreview>,
    reactions: Vec<PreviewReaction>,
}

/// Return only the referenced message and its reactions, never a channel's history.
async fn notification_context(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid notification ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let unavailable = || AppError::NotFound("Notification context unavailable".into());
    let notification = Notification::filter(
        Notification::fields()
            .id()
            .eq(id.as_str())
            .and(Notification::fields().user_id().eq(actor.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .ok_or_else(unavailable)?;
    let channel = Channel::filter(Channel::fields().id().eq(notification.channel_id.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .ok_or_else(unavailable)?;
    if channel.deleted_at.is_some() {
        return Err(unavailable());
    }
    if channel.is_private || channel.is_dm || crate::apps::events::guest(&mut tx, &actor.id).await?
    {
        let member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(channel.id.as_str())
                .and(ChannelUser::fields().user_id().eq(actor.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;
        if member.is_none() {
            return Err(unavailable());
        }
    }
    let message = Message::filter(
        Message::fields()
            .id()
            .eq(notification.message_id.as_str())
            .and(Message::fields().channel_id().eq(channel.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .filter(|message| message.deleted_at.is_none());
    let reactions = if message.is_some() && notification.notification_type == "reaction" {
        MessageReaction::filter(
            MessageReaction::fields()
                .message_id()
                .eq(notification.message_id.as_str()),
        )
        .exec(&mut tx)
        .await?
        .into_iter()
        .map(|row| PreviewReaction {
            user_id: row.user_id,
            emoji: row.emoji,
        })
        .collect()
    } else {
        Vec::new()
    };
    let context = NotificationContext {
        // Include one extra character so the client can indicate that the preview was truncated.
        message: message.map(|row| MessagePreview {
            id: row.id,
            user_id: row.user_id,
            content: row.content.chars().take(281).collect(),
        }),
        reactions,
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(context)).into_response())
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
