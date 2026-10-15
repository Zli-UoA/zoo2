use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser},
    routes::response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub id: Option<String>,
    pub channel_name: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub channel_name: Option<String>,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResponse {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: bool,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub archive_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl From<Channel> for ChannelResponse {
    fn from(c: Channel) -> Self {
        Self {
            id: c.id,
            channel_name: c.channel_name,
            description: c.description,
            created_user_id: c.created_user_id,
            is_private: c.is_private,
            created_at: c.created_at.to_string(),
            updated_at: c.updated_at.map(|t| t.to_string()),
            archive_at: c.archive_at.map(|t| t.to_string()),
            deleted_at: c.deleted_at.map(|t| t.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUserResponse {
    pub user_id: String,
    pub channel_id: String,
    pub joined_at: String,
}

impl From<ChannelUser> for ChannelUserResponse {
    fn from(cu: ChannelUser) -> Self {
        Self {
            user_id: cu.user_id,
            channel_id: cu.channel_id,
            joined_at: cu.joined_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveChannelResponse {
    pub user_id: String,
    pub channel_id: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels", post(create_channel).get(list_channels))
        .route(
            "/api/channels/{id}",
            get(get_channel_by_id)
                .patch(update_channel)
                .delete(delete_channel),
        )
        .route("/api/channels/{id}/join", post(join_channel))
        .route("/api/channels/{id}/leave", post(leave_channel))
}

async fn create_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateChannelRequest>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("channel id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let now = crate::models::now_utc_millis();

    let channel = toasty::create!(Channel {
        id: id.as_str(),
        channel_name: payload.channel_name.as_str(),
        description: payload.description,
        created_user_id: auth_user.id.as_str(),
        is_private: payload.is_private.unwrap_or(false),
        created_at: now,
        updated_at: None,
        archive_at: None,
        deleted_at: None,
    })
    .exec(&mut tx)
    .await?;

    // 作成者をチャンネルメンバーに追加
    toasty::create!(ChannelUser {
        user_id: auth_user.id.as_str(),
        channel_id: id.as_str(),
        joined_at: now,
    })
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(ChannelResponse::from(channel), txid)))
}

async fn update_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("channel id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let channel = Channel::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::BadRequest("Cannot edit a deleted channel".into()));
    }

    if channel.created_user_id != auth_user.id {
        return Err(AppError::Forbidden(
            "Only the channel creator can update this channel".into(),
        ));
    }

    let now = crate::models::now_utc_millis();
    let mut update_stmt = Channel::update_by_id(id.as_str()).updated_at(Some(now));

    if let Some(ref name) = payload.channel_name {
        update_stmt = update_stmt.channel_name(name.as_str());
    }
    if let Some(ref desc) = payload.description {
        update_stmt = update_stmt.description(Some(desc.clone()));
    }
    if let Some(is_private) = payload.is_private {
        update_stmt = update_stmt.is_private(is_private);
    }

    update_stmt.exec(&mut tx).await?;

    let updated_channel = Channel::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ChannelResponse::from(updated_channel),
        txid,
    )))
}

async fn delete_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("channel id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let channel = Channel::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.created_user_id != auth_user.id {
        return Err(AppError::Forbidden(
            "Only the channel creator can delete this channel".into(),
        ));
    }

    let now = crate::models::now_utc_millis();
    Channel::update_by_id(id.as_str())
        .deleted_at(Some(now))
        .exec(&mut tx)
        .await?;

    let deleted_channel = Channel::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ChannelResponse::from(deleted_channel),
        txid,
    )))
}

async fn join_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ChannelUserResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("channel id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    let channel = Channel::get_by_id(&mut tx, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::BadRequest("Cannot join a deleted channel".into()));
    }

    let existing = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?;

    let channel_user = if let Some(cu) = existing {
        cu
    } else {
        let now = crate::models::now_utc_millis();
        toasty::create!(ChannelUser {
            user_id: auth_user.id.as_str(),
            channel_id: id.as_str(),
            joined_at: now,
        })
        .exec(&mut tx)
        .await?
    };

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ChannelUserResponse::from(channel_user),
        txid,
    )))
}

async fn leave_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<LeaveChannelResponse>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| {
        AppError::BadRequest(format!("channel id is not a valid UUID: {id}"))
    })?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .delete()
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        LeaveChannelResponse {
            user_id: auth_user.id,
            channel_id: id,
        },
        txid,
    )))
}

async fn list_channels(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ChannelResponse>>, AppError> {
    let mut db = state.db.clone();

    // ユーザーが参加しているチャンネル ID リストを取得
    let memberships = ChannelUser::filter(ChannelUser::fields().user_id().eq(auth_user.id.as_str()))
        .exec(&mut db)
        .await?;
    let joined_channel_ids: Vec<String> = memberships.into_iter().map(|m| m.channel_id).collect();

    let all_channels = Channel::filter(Channel::fields().deleted_at().is_none())
        .order_by(Channel::fields().created_at().asc())
        .exec(&mut db)
        .await?;

    let visible_channels: Vec<ChannelResponse> = all_channels
        .into_iter()
        .filter(|c| !c.is_private || joined_channel_ids.contains(&c.id))
        .map(ChannelResponse::from)
        .collect();

    Ok(Json(visible_channels))
}

async fn get_channel_by_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ChannelResponse>, AppError> {
    let mut db = state.db.clone();
    let channel = Channel::get_by_id(&mut db, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found with id: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!("Channel not found with id: {id}")));
    }

    if channel.is_private {
        let is_member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?
        .is_some();

        if !is_member {
            return Err(AppError::Forbidden(
                "You do not have access to this private channel".into(),
            ));
        }
    }

    Ok(Json(ChannelResponse::from(channel)))
}
