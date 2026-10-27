use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{
        AuthUser,
        rbac::{
            can_add_member, can_join, can_kick_member, can_manage_channel, is_channel_visible,
            is_workspace_admin,
        },
    },
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser},
    routes::response::ApiResponse,
    state::AppState,
    validation::{validate_channel_description, validate_channel_name},
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

#[derive(Debug, Deserialize)]
pub struct AddChannelUserRequest {
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResponse {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: bool,
    pub is_dm: bool,
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
            is_dm: c.is_dm,
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
        .route("/api/channels/{id}/archive", post(archive_channel))
        .route("/api/channels/{id}/unarchive", post(unarchive_channel))
        .route("/api/channels/{id}/users", post(add_channel_user))
        .route(
            "/api/channels/{id}/users/{user_id}",
            delete(kick_channel_user),
        )
}

async fn create_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateChannelRequest>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    validate_channel_name(&payload.channel_name)?;
    validate_channel_description(payload.description.as_deref())?;

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let now = crate::models::now_utc_millis();

    let channel = toasty::create!(Channel {
        id: id.as_str(),
        channel_name: payload.channel_name.as_str(),
        description: payload.description,
        created_user_id: auth_user.id.as_str(),
        is_private: payload.is_private.unwrap_or(false),
        is_dm: false,
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
    if let Some(ref name) = payload.channel_name {
        validate_channel_name(name)?;
    }
    if let Some(ref desc) = payload.description {
        validate_channel_description(Some(desc))?;
    }

    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    if channel.is_dm
        || !can_manage_channel(
            &channel.created_user_id,
            &auth_user.id,
            is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot update a direct message channel"
            } else {
                "Only the channel creator or a workspace admin can update this channel"
            }
            .into(),
        ));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot update an archived channel".into(),
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
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    if channel.is_dm
        || !can_manage_channel(
            &channel.created_user_id,
            &auth_user.id,
            is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot delete a direct message channel"
            } else {
                "Only the channel creator or a workspace admin can delete this channel"
            }
            .into(),
        ));
    }

    let now = crate::models::now_utc_millis();
    Channel::update_by_id(id.as_str())
        .deleted_at(Some(now))
        .updated_at(Some(now))
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
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot join an archived channel".into(),
        ));
    }

    // DM チャンネルへの直接 join は禁止。
    // かつ非参加者にはチャンネルの存在自体を推測させないため 404 Not Found を返却する。
    if channel.is_dm {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }

        return Err(AppError::Forbidden(
            "Cannot join a direct message channel".into(),
        ));
    }

    // プライベートチャンネルへの直接 join は禁止 (作成者または招待による追加のみ)
    if !can_join(channel.is_private, channel.is_dm) {
        return Err(AppError::Forbidden(
            "Cannot join a private channel directly. You must be added by the channel owner."
                .into(),
        ));
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

async fn add_channel_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<AddChannelUserRequest>,
) -> Result<Json<ApiResponse<ChannelUserResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;
    Uuid::parse_str(&payload.user_id).map_err(|_| {
        AppError::BadRequest(format!("user_id is not a valid UUID: {}", payload.user_id))
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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    let is_creator = channel.created_user_id == auth_user.id;
    let is_actor_member = if is_creator {
        true
    } else if !channel.is_private {
        ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?
        .is_some()
    } else {
        false
    };

    if channel.is_dm
        || !can_add_member(
            channel.is_private,
            &channel.created_user_id,
            &auth_user.id,
            is_actor_member,
            is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot add members to a direct message channel"
            } else if channel.is_private {
                "Only the channel creator or a workspace admin can add members to a private channel"
            } else {
                "You must be a member, creator, or workspace admin of the channel to add other users"
            }
            .into(),
        ));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot add members to an archived channel".into(),
        ));
    }

    let target = crate::models::User::get_by_id(&mut tx, payload.user_id.as_str()).await?;
    if target.is_bot {
        return Err(AppError::BadRequest(
            "Integration users cannot be added manually".into(),
        ));
    }

    let existing = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(payload.user_id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?;

    let channel_user = if let Some(cu) = existing {
        cu
    } else {
        let now = crate::models::now_utc_millis();
        toasty::create!(ChannelUser {
            user_id: payload.user_id.as_str(),
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
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    // 作成者が抜けると作成者不在のチャンネルになり、管理権限を失う。
    // UI でも退出ボタンを無効化しているが、API 直叩きに対する最終防衛線として拒否する。
    // ※ DM チャンネルでは作成者も自由に退出可能
    if !channel.is_dm && channel.created_user_id == auth_user.id {
        return Err(AppError::BadRequest(
            "The channel creator cannot leave the channel".into(),
        ));
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

    if existing.is_none() {
        return Err(AppError::NotFound(format!(
            "You are not a member of channel: {id}"
        )));
    }

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

    state
        .call_registry
        .remove_user_from_channel(&id, &auth_user.id)
        .await;

    Ok(Json(ApiResponse::new(
        LeaveChannelResponse {
            user_id: auth_user.id,
            channel_id: id,
        },
        txid,
    )))
}

async fn archive_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    if channel.is_dm
        || !can_manage_channel(
            &channel.created_user_id,
            &auth_user.id,
            is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot archive a direct message channel"
            } else {
                "Only the channel creator or a workspace admin can archive this channel"
            }
            .into(),
        ));
    }

    if channel.archive_at.is_some() {
        // no-op: WAL 書き込みがないため txid: None を返却
        return Ok(Json(ApiResponse::new(ChannelResponse::from(channel), None)));
    }

    let now = crate::models::now_utc_millis();
    Channel::update_by_id(id.as_str())
        .archive_at(Some(now))
        .updated_at(Some(now))
        .exec(&mut tx)
        .await?;

    let updated = Channel::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    state.call_registry.close_room(&id).await;

    Ok(Json(ApiResponse::new(ChannelResponse::from(updated), txid)))
}

async fn unarchive_channel(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    if channel.is_dm
        || !can_manage_channel(
            &channel.created_user_id,
            &auth_user.id,
            is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot unarchive a direct message channel"
            } else {
                "Only the channel creator or a workspace admin can unarchive this channel"
            }
            .into(),
        ));
    }

    if channel.archive_at.is_none() {
        // no-op: WAL 書き込みがないため txid: None を返却
        return Ok(Json(ApiResponse::new(ChannelResponse::from(channel), None)));
    }

    let now = crate::models::now_utc_millis();
    Channel::update_by_id(id.as_str())
        .archive_at(None)
        .updated_at(Some(now))
        .exec(&mut tx)
        .await?;

    let updated = Channel::get_by_id(&mut tx, id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(ChannelResponse::from(updated), txid)))
}

async fn kick_channel_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id, user_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<ChannelUserResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;
    Uuid::parse_str(&user_id)
        .map_err(|_| AppError::BadRequest(format!("user_id is not a valid UUID: {user_id}")))?;

    if auth_user.id == user_id {
        return Err(AppError::BadRequest(
            "Cannot kick yourself; use leave endpoint instead".into(),
        ));
    }

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
    }

    let actor_is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?
    };

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none()
            && (!actor_is_admin || channel.is_dm)
            && (channel.created_user_id != auth_user.id || channel.is_dm)
        {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    let target_is_admin = if channel.is_dm {
        false
    } else {
        is_workspace_admin(&mut tx, &user_id, state.config.dev_auth_enabled).await?
    };

    if channel.is_dm
        || !can_kick_member(
            &channel.created_user_id,
            &auth_user.id,
            &user_id,
            actor_is_admin,
            target_is_admin,
            channel.is_dm,
        )
    {
        return Err(AppError::Forbidden(
            if channel.is_dm {
                "Cannot kick members from a direct message channel"
            } else {
                "You do not have permission to kick this member"
            }
            .into(),
        ));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot kick member in an archived channel".into(),
        ));
    }

    let existing = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(user_id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("User is not a member of channel: {id}")))?;

    ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(user_id.as_str())),
    )
    .delete()
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    state
        .call_registry
        .remove_user_from_channel(&id, &user_id)
        .await;

    Ok(Json(ApiResponse::new(
        ChannelUserResponse::from(existing),
        txid,
    )))
}

async fn list_channels(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ChannelResponse>>, AppError> {
    let mut db = state.db.clone();

    // ユーザーが参加しているチャンネル ID リストを取得
    let memberships =
        ChannelUser::filter(ChannelUser::fields().user_id().eq(auth_user.id.as_str()))
            .exec(&mut db)
            .await?;
    let joined_channel_ids: Vec<String> = memberships.into_iter().map(|m| m.channel_id).collect();

    let all_channels = Channel::filter(Channel::fields().deleted_at().is_none())
        .order_by(Channel::fields().created_at().asc())
        .exec(&mut db)
        .await?;

    let visible_channels: Vec<ChannelResponse> = all_channels
        .into_iter()
        .filter(|c| is_channel_visible(c.is_private, joined_channel_ids.contains(&c.id)))
        .map(ChannelResponse::from)
        .collect();

    Ok(Json(visible_channels))
}

async fn get_channel_by_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ChannelResponse>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!(
            "Channel not found with id: {id}"
        )));
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

        // 存在推測攻撃を防ぐため 403 ではなく 404 を返却
        if !is_channel_visible(channel.is_private, is_member) {
            return Err(AppError::NotFound(format!(
                "Channel not found with id: {id}"
            )));
        }
    }

    Ok(Json(ChannelResponse::from(channel)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_add_member() {
        let creator_id = "creator-user-id";
        let member_id = "member-user-id";
        let non_member_id = "non-member-user-id";

        // 1. プライベートチャンネル
        // - 作成者は追加可能
        assert!(can_add_member(
            true, creator_id, creator_id, true, false, false
        ));
        assert!(can_add_member(
            true, creator_id, creator_id, false, false, false
        ));
        // - 管理者は追加可能
        assert!(can_add_member(
            true, creator_id, "admin-id", false, true, false
        ));
        // - メンバーであっても非作成者は追加不可
        assert!(!can_add_member(
            true, creator_id, member_id, true, false, false
        ));
        // - 未参加ユーザーは追加不可
        assert!(!can_add_member(
            true,
            creator_id,
            non_member_id,
            false,
            false,
            false,
        ));

        // 2. パブリックチャンネル
        // - 作成者は常に追加可能
        assert!(can_add_member(
            false, creator_id, creator_id, true, false, false
        ));
        assert!(can_add_member(
            false, creator_id, creator_id, false, false, false
        ));
        // - 既存メンバーは追加可能
        assert!(can_add_member(
            false, creator_id, member_id, true, false, false
        ));
        // - 未参加の非作成者は追加不可 (公開チャンネルに他人を勝手に追加できない)
        assert!(!can_add_member(
            false,
            creator_id,
            non_member_id,
            false,
            false,
            false,
        ));

        // 3. DM
        assert!(!can_add_member(
            true, creator_id, creator_id, true, true, true
        ));
    }

    #[test]
    fn test_can_join() {
        // パブリックチャンネルなら直接参加可能
        assert!(can_join(false, false));
        // プライベートチャンネルなら直接参加不可
        assert!(!can_join(true, false));
        // DMなら直接参加不可
        assert!(!can_join(false, true));
        assert!(!can_join(true, true));
    }

    #[test]
    fn test_can_manage_channel() {
        let creator_id = "creator-user-id";
        let other_id = "other-user-id";

        assert!(can_manage_channel(creator_id, creator_id, false, false));
        assert!(can_manage_channel(creator_id, other_id, true, false));
        assert!(!can_manage_channel(creator_id, other_id, false, false));
        // DM
        assert!(!can_manage_channel(creator_id, creator_id, true, true));
    }

    #[test]
    fn test_is_channel_visible() {
        // パブリックチャンネルは誰でも閲覧可能
        assert!(is_channel_visible(false, true));
        assert!(is_channel_visible(false, false));

        // プライベートチャンネルはメンバーのみ閲覧可能
        assert!(is_channel_visible(true, true));
        assert!(!is_channel_visible(true, false));
    }
}
