use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{
        AuthUser,
        rbac::{can_delete_message, is_workspace_admin},
    },
    db::get_current_txid,
    error::AppError,
    models::{Attachment, Channel, ChannelUser, Message, MessageReaction},
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
    pub attachment_ids: Option<Vec<String>>,
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
    pub card: Option<String>,
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
            card: m.card,
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
    auth_user: AuthUser,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    create_message_impl(state, auth_user, payload, true, None).await
}

pub(super) async fn create_message_impl(
    state: AppState,
    auth_user: AuthUser,
    payload: CreateMessageRequest,
    notify: bool,
    card: Option<String>,
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

    // 添付ファイルの検証 (外部ストレージへの HeadObject 検証はトランザクション外で実行)
    let attachment_ids = payload.attachment_ids.unwrap_or_default();
    const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
    if attachment_ids.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(AppError::BadRequest(format!(
            "Too many attachments: maximum allowed is {MAX_ATTACHMENTS_PER_MESSAGE}, got {}",
            attachment_ids.len()
        )));
    }

    let mut verified_attachments = Vec::new();
    if !attachment_ids.is_empty() {
        let mut seen = std::collections::HashSet::new();
        for att_id in &attachment_ids {
            Uuid::parse_str(att_id).map_err(|_| {
                AppError::BadRequest(format!("attachment id is not a valid UUID: {att_id}"))
            })?;
            if !seen.insert(att_id) {
                return Err(AppError::BadRequest(format!(
                    "Duplicate attachment_id in request: {att_id}"
                )));
            }

            let att = Attachment::get_by_id(&mut db, att_id.as_str())
                .await
                .map_err(|err| {
                    if err.is_record_not_found() {
                        AppError::NotFound(format!("Attachment not found: {att_id}"))
                    } else {
                        AppError::from(err)
                    }
                })?;

            if att.uploader_id != auth_user.id {
                return Err(AppError::Forbidden(format!(
                    "Cannot attach file uploaded by another user: {att_id}"
                )));
            }

            if att.channel_id != payload.channel_id {
                return Err(AppError::BadRequest(format!(
                    "Attachment channel mismatch: expected {}, got {}",
                    payload.channel_id, att.channel_id
                )));
            }

            if att.message_id.is_some() {
                return Err(AppError::BadRequest(format!(
                    "Attachment {att_id} is already attached to a message"
                )));
            }

            // HeadObject による実体検証
            crate::storage::verify_uploaded_object(
                &state.s3_client,
                &state.config.s3.bucket,
                &att.storage_key,
                att.file_size,
                &att.content_type,
            )
            .await?;

            verified_attachments.push(att.id);
        }
    }

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
    let mut parent_author_id: Option<String> = None;
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

        parent_author_id = Some(parent.user_id);
    }

    let now = crate::models::now_utc_millis();

    // 投稿者は必ず認証済みユーザー (auth_user.id) を強制
    let message = toasty::create!(Message {
        id: id.as_str(),
        user_id: auth_user.id.as_str(),
        channel_id: payload.channel_id.as_str(),
        content: payload.content.as_str(),
        card,
        created_at: now,
        updated_at: None,
        deleted_at: None,
        parent_id: payload.parent_id,
    })
    .exec(&mut tx)
    .await?;

    // 添付ファイルの紐付け (条件付き原子的 UPDATE)
    for att_id in &verified_attachments {
        let rows = toasty::sql::query(
            "UPDATE attachments SET message_id = $1
             WHERE id = $2 AND uploader_id = $3 AND channel_id = $4
               AND message_id IS NULL AND created_at > now() - INTERVAL '23 hours'
             RETURNING id",
        )
        .bind(&id)
        .bind(att_id)
        .bind(&auth_user.id)
        .bind(&payload.channel_id)
        .exec(&mut tx)
        .await?;

        if rows.len() != 1 {
            let _ = tx.rollback().await;
            return Err(AppError::BadRequest(format!(
                "Failed to attach file {att_id}: conflict, already attached, or expired"
            )));
        }
    }

    if notify {
        crate::notifications::create_message_notifications(
            &mut tx,
            &message,
            auth_user.id.as_str(),
            parent_author_id.as_deref(),
        )
        .await?;
    }

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

    if message.card.is_some() {
        return Err(AppError::Forbidden(
            "Integration cards cannot be edited through the user API".into(),
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
    auth_user: AuthUser,
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

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            message.channel_id
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
                .eq(message.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;

        // ワークスペース管理者でもなくメンバーでもない場合は存在隠蔽 (404)
        // DM の場合は管理者であってもメンバーでなければ存在隠蔽 (404)
        if membership.is_none() && (!is_admin || channel.is_dm) {
            return Err(AppError::NotFound(format!("Message not found: {id}")));
        }
    }

    if !can_delete_message(
        &message.user_id,
        &channel.created_user_id,
        &auth_user.id,
        is_admin,
        channel.is_dm,
    ) {
        return Err(AppError::Forbidden(
            "You do not have permission to delete this message".into(),
        ));
    }

    // アーカイブ中の削除制限: 管理者およびチャンネル作成者は例外として強制削除を許可 (DM の場合は管理者・作成者特権無効)
    if channel.archive_at.is_some()
        && (channel.is_dm || (!is_admin && auth_user.id != channel.created_user_id))
    {
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
    auth_user: AuthUser,
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
    auth_user: AuthUser,
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
        id: reaction_id.as_str(),
        message_id: id.as_str(),
        user_id: auth_user.id.as_str(),
        emoji: normalized_emoji.as_str(),
        created_at: now,
    })
    .exec(&mut tx)
    .await;

    let reaction = match created_res {
        Ok(r) => {
            crate::notifications::create_reaction_notification(
                &mut tx,
                &message,
                auth_user.id.as_str(),
            )
            .await?;
            r
        }
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
    auth_user: AuthUser,
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
