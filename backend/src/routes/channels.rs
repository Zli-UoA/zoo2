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

/// メンバー追加権限の純粋判定ロジック:
/// - 作成者は常に追加可能
/// - プライベートチャンネル: 作成者のみ追加可能
/// - パブリックチャンネル: 作成者または既存メンバーが追加可能
pub(crate) fn can_add_member(
    is_private: bool,
    channel_creator_id: &str,
    actor_id: &str,
    is_actor_member: bool,
) -> bool {
    if channel_creator_id == actor_id {
        return true;
    }
    if is_private { false } else { is_actor_member }
}

/// チャンネル直接参加 (join) の純粋判定ロジック:
/// - パブリックチャンネル: 誰でも直接 join 可能
/// - プライベートチャンネル: 直接 join 不可 (オーナーによる追加のみ)
pub(crate) fn can_join(is_private: bool) -> bool {
    !is_private
}

/// チャンネル更新・削除権限の純粋判定ロジック:
/// - 作成者のみ変更・削除可能
pub(crate) fn can_modify_channel(channel_creator_id: &str, actor_id: &str) -> bool {
    channel_creator_id == actor_id
}

/// チャンネル閲覧権限の純粋判定ロジック:
/// - パブリックチャンネル: 誰でも閲覧可能
/// - プライベートチャンネル: メンバーのみ閲覧可能
pub(crate) fn is_channel_visible(is_private: bool, is_member: bool) -> bool {
    !is_private || is_member
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
        .route("/api/channels/{id}/users", post(add_channel_user))
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

    if !can_modify_channel(&channel.created_user_id, &auth_user.id) {
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

    if !can_modify_channel(&channel.created_user_id, &auth_user.id) {
        return Err(AppError::Forbidden(
            "Only the channel creator can delete this channel".into(),
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

    // プライベートチャンネルへの直接 join は禁止 (作成者または招待による追加のみ)
    if !can_join(channel.is_private) {
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

    // メンバー追加権限のチェック:
    // - プライベートチャンネル: チャンネル作成者のみ追加可能
    // - パブリックチャンネル: チャンネル作成者または既存メンバーのみ追加可能
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

    if !can_add_member(
        channel.is_private,
        &channel.created_user_id,
        &auth_user.id,
        is_actor_member,
    ) {
        return Err(AppError::Forbidden(
            if channel.is_private {
                "Only the channel creator can add members to a private channel"
            } else {
                "You must be a member or creator of the channel to add other users"
            }
            .into(),
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
        assert!(can_add_member(true, creator_id, creator_id, true));
        assert!(can_add_member(true, creator_id, creator_id, false));
        // - メンバーであっても非作成者は追加不可
        assert!(!can_add_member(true, creator_id, member_id, true));
        // - 未参加ユーザーは追加不可
        assert!(!can_add_member(true, creator_id, non_member_id, false));

        // 2. パブリックチャンネル
        // - 作成者は常に追加可能
        assert!(can_add_member(false, creator_id, creator_id, true));
        assert!(can_add_member(false, creator_id, creator_id, false));
        // - 既存メンバーは追加可能
        assert!(can_add_member(false, creator_id, member_id, true));
        // - 未参加の非作成者は追加不可 (公開チャンネルに他人を勝手に追加できない)
        assert!(!can_add_member(false, creator_id, non_member_id, false));
    }

    #[test]
    fn test_can_join() {
        // パブリックチャンネルなら直接参加可能
        assert!(can_join(false));
        // プライベートチャンネルなら直接参加不可
        assert!(!can_join(true));
    }

    #[test]
    fn test_can_modify_channel() {
        let creator_id = "creator-user-id";
        let other_id = "other-user-id";

        assert!(can_modify_channel(creator_id, creator_id));
        assert!(!can_modify_channel(creator_id, other_id));
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
