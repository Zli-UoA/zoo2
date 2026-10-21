use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{
        rbac::{can_delete_message, is_workspace_admin},
        tenant::TenantAuth,
    },
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message, MessageReaction},
    routes::response::ApiResponse,
    state::AppState,
    validation::{normalize_and_validate_emoji, validate_message_content},
};

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub id: Option<String>,
    pub channel_id: String,
    pub content: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessageRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub channel_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AddReactionRequest {
    pub id: Option<String>,
    pub emoji: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteReactionQuery {
    pub emoji: String,
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
    pub parent_id: Option<String>,
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
            parent_id: m.parent_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionResponse {
    pub id: String,
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
    pub created_at: String,
}

impl From<MessageReaction> for ReactionResponse {
    fn from(r: MessageReaction) -> Self {
        Self {
            id: r.id,
            message_id: r.message_id,
            user_id: r.user_id,
            emoji: r.emoji,
            created_at: r.created_at.to_string(),
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
        .route(
            "/api/messages/{id}/reactions",
            post(add_reaction).delete(delete_reaction),
        )
}

async fn create_message(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    validate_message_content(&payload.content)?;

    Uuid::parse_str(&payload.channel_id).map_err(|_| {
        AppError::BadRequest(format!(
            "channel_id is not a valid UUID: {}",
            payload.channel_id
        ))
    })?;

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    // チャンネル存在確認
    let channel = Channel::get_by_id(&mut tx, payload.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", payload.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            payload.channel_id
        )));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot post message to an archived channel".into(),
        ));
    }

    // プライベートチャンネルの場合、所属していなければ 404 で存在を隠蔽
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
            return Err(AppError::NotFound(format!(
                "Channel not found: {}",
                payload.channel_id
            )));
        }
    }

    // スレッド親メッセージ検証
    if let Some(ref parent_id) = payload.parent_id {
        Uuid::parse_str(parent_id).map_err(|_| {
            AppError::BadRequest(format!("parent_id is not a valid UUID: {parent_id}"))
        })?;

        let parent = Message::get_by_id(&mut tx, parent_id.as_str())
            .await
            .map_err(|err| {
                if err.is_record_not_found() {
                    AppError::NotFound(format!("Parent message not found: {parent_id}"))
                } else {
                    AppError::from(err)
                }
            })?;

        if parent.deleted_at.is_some()
            || parent.channel_id != payload.channel_id
            || parent_id == &id
        {
            return Err(AppError::NotFound(format!(
                "Parent message not found: {parent_id}"
            )));
        }

        // Slack 準拠: スレッド返信に対する再返信（木構造ネスト）は禁止
        if parent.parent_id.is_some() {
            return Err(AppError::BadRequest(
                "Nested thread replies are not supported; reply to the parent message directly"
                    .into(),
            ));
        }
    }

    let now = crate::models::now_utc_millis();

    // 投稿者は必ず認証済みユーザー (auth_user.id) を強制
    let message = toasty::create!(Message {
        tenant_id: auth_user.tenant_id.as_str(),
        id: id.as_str(),
        user_id: auth_user.id.as_str(),
        channel_id: payload.channel_id.as_str(),
        content: payload.content.as_str(),
        created_at: now,
        updated_at: None,
        deleted_at: None,
        parent_id: payload.parent_id,
    })
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(MessageResponse::from(message), txid)))
}

async fn update_message(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    validate_message_content(&payload.content)?;

    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!("Message not found: {id}")));
    }

    let channel = Channel::get_by_id(&mut tx, message.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", message.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            message.channel_id
        )));
    }

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(message.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!("Message not found: {id}")));
        }
    }

    if message.user_id != auth_user.id {
        return Err(AppError::Forbidden(
            "Only the message author can edit this message".into(),
        ));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot edit message in an archived channel".into(),
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
    auth_user: TenantAuth,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

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
        return Err(AppError::NotFound(format!("Message not found: {id}")));
    }

    let channel = Channel::get_by_id(&mut tx, message.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", message.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            message.channel_id
        )));
    }

    let is_admin = is_workspace_admin(
        &mut tx,
        &auth_user.id,
        &auth_user.tenant_id,
        state.config.dev_auth_enabled,
    )
    .await?;

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(message.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        // ワークスペース管理者でもなくメンバーでもない場合は存在隠蔽 (404)
        if membership.is_none() && !is_admin {
            return Err(AppError::NotFound(format!("Message not found: {id}")));
        }
    }

    if !can_delete_message(
        &message.user_id,
        &channel.created_user_id,
        &auth_user.id,
        is_admin,
    ) {
        return Err(AppError::Forbidden(
            "You do not have permission to delete this message".into(),
        ));
    }

    // アーカイブ中の削除制限: 管理者およびチャンネル作成者は例外として強制削除を許可
    if channel.archive_at.is_some() && !is_admin && auth_user.id != channel.created_user_id {
        return Err(AppError::BadRequest(
            "Cannot delete message in an archived channel".into(),
        ));
    }

    let now = crate::models::now_utc_millis();

    Message::update_by_id(id.as_str())
        .deleted_at(Some(now))
        .updated_at(Some(now))
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
    auth_user: TenantAuth,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<MessageResponse>>, AppError> {
    let mut db = state.db.clone();

    Uuid::parse_str(&query.channel_id).map_err(|_| {
        AppError::BadRequest(format!(
            "channel_id is not a valid UUID: {}",
            query.channel_id
        ))
    })?;

    let channel = Channel::get_by_id(&mut db, query.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", query.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            query.channel_id
        )));
    }

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(query.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!(
                "Channel not found: {}",
                query.channel_id
            )));
        }
    }

    let messages = Message::filter(
        Message::fields()
            .channel_id()
            .eq(query.channel_id.as_str())
            .and(Message::fields().deleted_at().is_none()),
    )
    .order_by(Message::fields().created_at().asc())
    .exec(&mut db)
    .await?;

    let res = messages.into_iter().map(MessageResponse::from).collect();
    Ok(Json(res))
}

async fn add_reaction(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Path(id): Path<String>,
    Json(payload): Json<AddReactionRequest>,
) -> Result<Json<ApiResponse<ReactionResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let normalized_emoji = normalize_and_validate_emoji(&payload.emoji)?;

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
        return Err(AppError::NotFound(format!("Message not found: {id}")));
    }

    let channel = Channel::get_by_id(&mut tx, message.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", message.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            message.channel_id
        )));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot add reaction in an archived channel".into(),
        ));
    }

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(message.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!("Message not found: {id}")));
        }
    }

    let reaction_id = if let Some(req_id) = payload.id {
        Uuid::parse_str(&req_id).map_err(|_| {
            AppError::BadRequest(format!("reaction id is not a valid UUID: {req_id}"))
        })?;
        req_id
    } else {
        Uuid::new_v4().to_string()
    };

    // 重複チェック
    let existing = MessageReaction::filter(
        MessageReaction::fields()
            .message_id()
            .eq(id.as_str())
            .and(
                MessageReaction::fields()
                    .user_id()
                    .eq(auth_user.id.as_str()),
            )
            .and(
                MessageReaction::fields()
                    .emoji()
                    .eq(normalized_emoji.as_str()),
            ),
    )
    .first()
    .exec(&mut tx)
    .await?;

    if let Some(reaction) = existing {
        // no-op: WAL 書き込みがないため txid: None を返却
        return Ok(Json(ApiResponse::new(
            ReactionResponse::from(reaction),
            None,
        )));
    }

    let now = crate::models::now_utc_millis();

    let created_res = toasty::create!(MessageReaction {
        tenant_id: auth_user.tenant_id.as_str(),
        id: reaction_id.as_str(),
        message_id: id.as_str(),
        user_id: auth_user.id.as_str(),
        emoji: normalized_emoji.as_str(),
        created_at: now,
    })
    .exec(&mut tx)
    .await;

    let reaction = match created_res {
        Ok(r) => r,
        Err(err) => {
            // UNIQUE制約違反等でトランザクションがabortされた場合、ロールバックして新しいtxで取得
            let _ = tx.rollback().await;
            let mut retry_tx = db.transaction().await?;
            let check = MessageReaction::filter(
                MessageReaction::fields()
                    .message_id()
                    .eq(id.as_str())
                    .and(
                        MessageReaction::fields()
                            .user_id()
                            .eq(auth_user.id.as_str()),
                    )
                    .and(
                        MessageReaction::fields()
                            .emoji()
                            .eq(normalized_emoji.as_str()),
                    ),
            )
            .first()
            .exec(&mut retry_tx)
            .await?;

            if let Some(r) = check {
                return Ok(Json(ApiResponse::new(ReactionResponse::from(r), None)));
            }

            // 指定された reaction_id が既に存在する場合は PK 衝突として 409 Conflict を返す
            let pk_check =
                MessageReaction::filter(MessageReaction::fields().id().eq(reaction_id.as_str()))
                    .first()
                    .exec(&mut retry_tx)
                    .await?;
            if pk_check.is_some() {
                return Err(AppError::Conflict(format!(
                    "Reaction with ID '{reaction_id}' already exists"
                )));
            }

            tracing::error!(?err, "Failed to create reaction");
            return Err(AppError::Internal(anyhow::anyhow!(
                "Failed to create reaction: {err}"
            )));
        }
    };

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ReactionResponse::from(reaction),
        txid,
    )))
}

async fn delete_reaction(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Path(id): Path<String>,
    Query(query): Query<DeleteReactionQuery>,
) -> Result<Json<ApiResponse<ReactionResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let normalized_emoji = normalize_and_validate_emoji(&query.emoji)?;

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
        return Err(AppError::NotFound(format!("Message not found: {id}")));
    }

    let channel = Channel::get_by_id(&mut tx, message.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", message.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    auth_user.check_channel(&channel)?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            message.channel_id
        )));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot remove reaction in an archived channel".into(),
        ));
    }

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(message.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!("Message not found: {id}")));
        }
    }

    let reaction = MessageReaction::filter(
        MessageReaction::fields()
            .message_id()
            .eq(id.as_str())
            .and(
                MessageReaction::fields()
                    .user_id()
                    .eq(auth_user.id.as_str()),
            )
            .and(
                MessageReaction::fields()
                    .emoji()
                    .eq(normalized_emoji.as_str()),
            ),
    )
    .first()
    .exec(&mut tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Reaction not found: {}", query.emoji)))?;

    MessageReaction::filter(MessageReaction::fields().id().eq(reaction.id.as_str()))
        .delete()
        .exec(&mut tx)
        .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        ReactionResponse::from(reaction),
        txid,
    )))
}
