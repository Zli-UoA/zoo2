use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message},
    routes::response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub id: Option<String>,
    pub user_id: Option<String>,
    pub channel_id: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessageRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    pub id: String,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl From<Message> for MessageResponse {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            channel_id: m.channel_id,
            content: m.content,
            created_at: m.created_at.to_string(),
            updated_at: m.updated_at.map(|t| t.to_string()),
            deleted_at: m.deleted_at.map(|t| t.to_string()),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/messages", post(create_message).get(list_messages))
        .route(
            "/api/messages/{id}",
            patch(update_message).delete(delete_message),
        )
}

async fn create_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    // ID バリデーション
    Uuid::parse_str(&payload.channel_id).map_err(|_| {
        AppError::BadRequest(format!("channel_id is not a valid UUID: {}", payload.channel_id))
    })?;

    let user_id = match payload.user_id {
        Some(uid) => {
            Uuid::parse_str(&uid).map_err(|_| {
                AppError::BadRequest(format!("user_id is not a valid UUID: {uid}"))
            })?;
            uid
        }
        None => auth_user.id.clone(),
    };

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("message id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    // チャンネル存在確認およびプライベート認可チェック
    let channel = Channel::get_by_id(&mut tx, payload.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", payload.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(payload.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none() {
            return Err(AppError::Forbidden(
                "You do not have access to this private channel".into(),
            ));
        }
    }

    let now = crate::models::now_utc_millis();

    let message = toasty::create!(Message {
        id: id.as_str(),
        user_id: user_id.as_str(),
        channel_id: payload.channel_id.as_str(),
        content: payload.content.as_str(),
        created_at: now,
        updated_at: None,
        deleted_at: None,
    })
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(MessageResponse::from(message), txid)))
}

async fn update_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("message id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let message = Message::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Message not found: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if message.deleted_at.is_some() {
        return Err(AppError::BadRequest("Cannot edit a deleted message".into()));
    }

    if message.user_id != auth_user.id {
        return Err(AppError::Forbidden(
            "Only the message author can edit this message".into(),
        ));
    }

    let now = crate::models::now_utc_millis();

    Message::update_by_id(id.as_str())
        .content(payload.content.as_str())
        .updated_at(Some(now))
        .exec(&mut tx)
        .await?;

    let updated_message = Message::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        MessageResponse::from(updated_message),
        txid,
    )))
}

async fn delete_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("message id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let message = Message::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Message not found: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if message.user_id != auth_user.id {
        return Err(AppError::Forbidden(
            "Only the message author can delete this message".into(),
        ));
    }

    let now = crate::models::now_utc_millis();

    Message::update_by_id(id.as_str())
        .deleted_at(Some(now))
        .exec(&mut tx)
        .await?;

    let deleted_message = Message::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        MessageResponse::from(deleted_message),
        txid,
    )))
}

async fn list_messages(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<MessageResponse>>, AppError> {
    let mut db = state.db.clone();

    if let Some(ref channel_id) = query.channel_id {
        Uuid::parse_str(channel_id).map_err(|_| {
            AppError::BadRequest(format!("channel_id is not a valid UUID: {channel_id}"))
        })?;

        let channel = Channel::get_by_id(&mut db, channel_id.as_str())
            .await
            .map_err(|err| {
                if err.is_record_not_found() {
                    AppError::NotFound(format!("Channel not found: {channel_id}"))
                } else {
                    AppError::from(err)
                }
            })?;

        if channel.is_private {
            let membership = ChannelUser::filter(
                ChannelUser::fields()
                    .channel_id()
                    .eq(channel_id.as_str())
                    .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
            )
            .first()
            .exec(&mut db)
            .await?;

            if membership.is_none() {
                return Err(AppError::Forbidden(
                    "You do not have access to this private channel".into(),
                ));
            }
        }
    }

    let messages = if let Some(channel_id) = query.channel_id {
        Message::filter(
            Message::fields()
                .channel_id()
                .eq(channel_id.as_str())
                .and(Message::fields().deleted_at().is_none()),
        )
        .order_by(Message::fields().created_at().asc())
        .exec(&mut db)
        .await?
    } else {
        Message::filter(Message::fields().deleted_at().is_none())
            .order_by(Message::fields().created_at().asc())
            .exec(&mut db)
            .await?
    };

    let res = messages.into_iter().map(MessageResponse::from).collect();
    Ok(Json(res))
}
