use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
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
    routes::{e2ee_channels, response::ApiResponse},
    state::AppState,
    validation::{normalize_and_validate_emoji, validate_message_content_with_options},
};

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub id: Option<String>,
    pub channel_id: String,
    pub content: String,
    pub parent_id: Option<String>,
    pub attachment_ids: Option<Vec<String>>,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub sensitive_attachment_ids: Vec<String>,
    #[serde(default = "default_message_kind")]
    pub kind: String,
    #[serde(default)]
    pub broadcast: bool,
    pub metadata: Option<serde_json::Value>,
    /// 暗号化チャンネルの Megolm の暗号文（`content` は空にする。docs/e2ee.md）。
    #[serde(default)]
    pub ciphertext: Option<serde_json::Value>,
    /// 暗号化したセッションの `membership_epoch`。
    #[serde(default)]
    pub epoch: Option<i64>,
    /// 作成では 0 だけ（省略できる）。
    #[serde(default)]
    pub revision: Option<i64>,
}

fn default_message_kind() -> String {
    "message".into()
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessageRequest {
    pub content: String,
    #[serde(default)]
    pub ciphertext: Option<serde_json::Value>,
    #[serde(default)]
    pub epoch: Option<i64>,
    /// 暗号化チャンネルでは必須で、今の値 + 1。平文チャンネルでは省略できる（送ったときだけ照合する）。
    #[serde(default)]
    pub revision: Option<i64>,
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
    #[serde(default)]
    pub apub: bool,
    pub apub_visibility: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
    pub parent_id: Option<String>,
    /// 暗号化チャンネルの暗号文（平文チャンネルと削除済みの行は null）。
    #[serde(default)]
    pub ciphertext: Option<serde_json::Value>,
    #[serde(default)]
    pub epoch: Option<i64>,
    #[serde(default)]
    pub revision: i64,
}

impl From<Message> for MessageResponse {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            channel_id: m.channel_id,
            content: m.content,
            card: m.card,
            apub: m.apub,
            apub_visibility: m.apub_visibility,
            created_at: m.created_at.to_string(),
            updated_at: m.updated_at.map(|t| t.to_string()),
            deleted_at: m.deleted_at.map(|t| t.to_string()),
            parent_id: m.parent_id,
            // toasty のモデルに含めない列。fill_e2ee_fields で埋める。
            ciphertext: None,
            epoch: None,
            revision: 0,
        }
    }
}

/// `ciphertext`、`epoch`、`revision`（toasty のモデルに含めない列）を DB から読んで埋める。
pub(crate) async fn fill_e2ee_fields(
    executor: &mut dyn toasty::Executor,
    messages: &mut [MessageResponse],
) -> Result<(), AppError> {
    if messages.is_empty() {
        return Ok(());
    }
    let ids = serde_json::Value::from(messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
        .to_string();
    let rows = toasty::sql::query(
        "SELECT json_build_object('id', id, 'ciphertext', ciphertext, 'epoch', epoch, 'revision', revision)::text
         FROM messages WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb))",
    )
    .bind(ids.as_str())
    .exec(executor)
    .await?;
    let mut fields = HashMap::new();
    for row in rows {
        if let toasty::stmt::Value::Record(r) = row
            && let Some(toasty::stmt::Value::String(text)) = r.fields.into_iter().next()
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(id) = value["id"].as_str()
        {
            fields.insert(id.to_owned(), value);
        }
    }
    for message in messages.iter_mut() {
        if let Some(value) = fields.get(&message.id) {
            message.ciphertext = Some(value["ciphertext"].clone()).filter(|v| !v.is_null());
            message.epoch = value["epoch"].as_i64();
            message.revision = value["revision"].as_i64().unwrap_or(0);
        }
    }
    Ok(())
}

async fn with_e2ee_fields(
    executor: &mut dyn toasty::Executor,
    message: MessageResponse,
) -> Result<MessageResponse, AppError> {
    let mut messages = [message];
    fill_e2ee_fields(executor, &mut messages).await?;
    let [message] = messages;
    Ok(message)
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

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Notify;

static TEST_SYNC_GATES: LazyLock<Mutex<HashMap<String, Arc<Notify>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_or_create_test_gate(key: &str) -> Arc<Notify> {
    let mut map = TEST_SYNC_GATES.lock().unwrap();
    map.entry(key.to_string())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

async fn create_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<(HeaderMap, Json<ApiResponse<MessageResponse>>), AppError> {
    let test_delay_ms = if state.config.dev_auth_enabled {
        headers
            .get("X-Test-Delay-Attachment-Verify")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|d| d.min(1000))
    } else {
        None
    };
    let test_gate_wait = if state.config.dev_auth_enabled {
        headers
            .get("X-Test-Gate-Wait")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    } else {
        None
    };
    let test_gate_notify = if state.config.dev_auth_enabled {
        headers
            .get("X-Test-Gate-Notify")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    } else {
        None
    };
    create_message_impl(
        state,
        auth_user,
        payload,
        true,
        None,
        test_delay_ms,
        test_gate_wait,
        test_gate_notify,
    )
    .await
}

pub(crate) async fn check_idempotent_existing_message(
    db: &mut toasty::Db,
    id: &str,
    auth_user_id: &str,
    channel_id: &str,
    content: &str,
    parent_id: Option<&str>,
    kind: &str,
    broadcast: bool,
    metadata: Option<&serde_json::Value>,
    attachment_ids: &[String],
) -> Result<Option<MessageResponse>, AppError> {
    let existing = Message::filter(Message::fields().id().eq(id))
        .first()
        .exec(db)
        .await?;
    let Some(existing_msg) = existing else {
        return Ok(None);
    };

    // 他チャンネルのメッセージ UUID を探る攻撃を防ぐため、channel_id が異なる場合は 404 を返す
    if existing_msg.channel_id != channel_id {
        return Err(AppError::NotFound(format!(
            "Channel not found: {channel_id}"
        )));
    }

    // 暗号化チャンネルでは、同じ id、user_id、channel_id の行があれば、新しい暗号文を保存せずに既存の行を返す
    // （応答をなくした再送。暗号文は送り直すたびに変わるので、内容では比べない）。
    if Channel::get_by_id(&mut *db, channel_id).await?.encrypted {
        if existing_msg.user_id == auth_user_id {
            return Ok(Some(
                with_e2ee_fields(&mut *db, MessageResponse::from(existing_msg)).await?,
            ));
        }
        return Err(AppError::Conflict("Idempotency key already used".into()));
    }

    let mut req_att_ids = attachment_ids.to_vec();
    req_att_ids.sort();

    let existing_atts = Attachment::filter(Attachment::fields().message_id().eq(id))
        .exec(db)
        .await?;
    let mut existing_att_ids: Vec<String> = existing_atts.into_iter().map(|a| a.id).collect();
    existing_att_ids.sort();

    let opt_rows = toasty::sql::query(
        "SELECT json_build_object(
            'kind', COALESCE(message_kind, 'message'),
            'broadcast', COALESCE(broadcast, false),
            'metadata', CASE WHEN metadata IS NOT NULL AND metadata <> '' THEN metadata::jsonb ELSE NULL END
         )::text FROM messages WHERE id = $1",
    )
    .bind(id)
    .exec(db)
    .await?;

    let mut options_match = false;
    if let Some(toasty::stmt::Value::Record(r)) = opt_rows.into_iter().next()
        && let Some(toasty::stmt::Value::String(s)) = r.fields.into_iter().next()
        && let Ok(saved_opts) = serde_json::from_str::<serde_json::Value>(&s)
    {
        let saved_kind = saved_opts["kind"].as_str().unwrap_or("message");
        let saved_broadcast = saved_opts["broadcast"].as_bool().unwrap_or(false);
        let saved_metadata = saved_opts.get("metadata").filter(|v| !v.is_null()).cloned();
        let req_metadata = metadata.filter(|v| !v.is_null()).cloned();

        options_match =
            saved_kind == kind && saved_broadcast == broadcast && saved_metadata == req_metadata;
    }

    if existing_msg.user_id == auth_user_id
        && existing_msg.channel_id == channel_id
        && existing_msg.content == content
        && existing_msg.parent_id.as_deref() == parent_id
        && existing_msg.deleted_at.is_none()
        && req_att_ids == existing_att_ids
        && options_match
    {
        return Ok(Some(MessageResponse::from(existing_msg)));
    }
    Err(AppError::Conflict("Idempotency key already used".into()))
}

pub(crate) async fn create_message_impl(
    state: AppState,
    auth_user: AuthUser,
    payload: CreateMessageRequest,
    notify: bool,
    card: Option<String>,
    test_delay_ms: Option<u64>,
    test_gate_wait: Option<String>,
    test_gate_notify: Option<String>,
) -> Result<(HeaderMap, Json<ApiResponse<MessageResponse>>), AppError> {
    let attachment_ids = payload.attachment_ids.unwrap_or_default();
    const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
    if attachment_ids.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(AppError::BadRequest(format!(
            "Too many attachments: maximum allowed is {MAX_ATTACHMENTS_PER_MESSAGE}, got {}",
            attachment_ids.len()
        )));
    }

    if payload
        .sensitive_attachment_ids
        .iter()
        .any(|id| !attachment_ids.contains(id))
    {
        return Err(AppError::BadRequest(
            "Sensitive attachment IDs must belong to this message".into(),
        ));
    }
    if !matches!(payload.kind.as_str(), "message" | "emote")
        || payload.broadcast && payload.parent_id.is_none()
    {
        return Err(AppError::BadRequest(
            "Invalid message kind or broadcast target".into(),
        ));
    }
    if let Some(metadata) = &payload.metadata {
        crate::apps::events::object(metadata)?;
    }

    Uuid::parse_str(&payload.channel_id).map_err(|_| {
        AppError::BadRequest(format!(
            "channel_id is not a valid UUID: {}",
            payload.channel_id
        ))
    })?;

    let supplied_id = payload.id.clone();
    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();

    // チャンネル存在確認・所属確認（他人のUUIDの存在漏洩防止のため冪等性チェックより前に検証）
    let channel = Channel::get_by_id(&mut db, payload.channel_id.as_str())
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

    // ユーザー属性（bot, active, guest）を取得
    let principal_rows = toasty::sql::query(
        "SELECT json_build_object(
            'bot', EXISTS(SELECT 1 FROM users WHERE id = $1 AND is_bot),
            'active', EXISTS(SELECT 1 FROM apps a JOIN app_installations i ON i.app_id = a.id WHERE a.bot_user_id = $1 AND a.status = 'installed' AND i.active),
            'guest', EXISTS(SELECT 1 FROM users WHERE id = $1 AND is_guest)
        )::text",
    )
    .bind(&auth_user.id)
    .exec(&mut db)
    .await?;

    let mut bot_actor = false;
    let mut guest_actor = false;
    if let Some(toasty::stmt::Value::Record(r)) = principal_rows.into_iter().next()
        && let Some(toasty::stmt::Value::String(s)) = r.fields.into_iter().next()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
    {
        bot_actor = v["bot"] == true;
        if bot_actor && v["active"] != true {
            return Err(AppError::Forbidden("App is not installed".into()));
        }
        guest_actor = v["guest"] == true;
    }

    // プライベートチャンネル、Bot、またはゲストの場合、チャンネルに所属していなければ 404 で存在を隠蔽
    // （他人の UUID の存在漏洩防止のため冪等性チェックおよびアーカイブ確認より前に検証）
    if channel.is_private || bot_actor || guest_actor {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(payload.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!(
                "Channel not found: {}",
                payload.channel_id
            )));
        }
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot post message to an archived channel".into(),
        ));
    }

    // 暗号化チャンネルは content を空にして ciphertext を送る。平文チャンネルに暗号化の項目は送れない。
    let sender = if channel.encrypted {
        if !payload.content.is_empty() {
            return Err(e2ee_channels::plaintext_forbidden());
        }
        let Some(ciphertext) = payload.ciphertext.as_ref().filter(|v| !v.is_null()) else {
            return Err(e2ee_channels::ciphertext_required());
        };
        if !attachment_ids.is_empty() || !payload.sensitive_attachment_ids.is_empty() {
            return Err(e2ee_channels::attachments_forbidden());
        }
        if payload.metadata.is_some() {
            return Err(AppError::BadRequest(
                "metadata cannot be used in an encrypted channel".into(),
            ));
        }
        if payload.epoch.is_none() {
            return Err(AppError::BadRequest(
                "epoch is required in an encrypted channel".into(),
            ));
        }
        if payload.revision.is_some_and(|r| r != 0) {
            return Err(AppError::BadRequest(
                "revision must be 0 for a new message".into(),
            ));
        }
        Some(e2ee_channels::check_ciphertext_shape(ciphertext)?)
    } else {
        if payload.ciphertext.is_some() || payload.epoch.is_some() || payload.revision.is_some() {
            return Err(e2ee_channels::plaintext_forbidden());
        }
        validate_message_content_with_options(&payload.content, !attachment_ids.is_empty())?;
        None
    };

    // クライアント指定の ID が既に存在する場合、同一属性（本文・チャンネル・親・添付ファイル）の再送であれば既存メッセージを返して成功とする (冪等性)
    if supplied_id.is_some() {
        if let Some(msg_res) = check_idempotent_existing_message(
            &mut db,
            &id,
            &auth_user.id,
            &payload.channel_id,
            &payload.content,
            payload.parent_id.as_deref(),
            &payload.kind,
            payload.broadcast,
            payload.metadata.as_ref(),
            &attachment_ids,
        )
        .await?
        {
            return Ok((HeaderMap::new(), Json(ApiResponse::new(msg_res, None))));
        }
    }

    if let Some(gate_key) = test_gate_wait {
        let gate = get_or_create_test_gate(&gate_key);
        let _ =
            tokio::time::timeout(tokio::time::Duration::from_millis(1000), gate.notified()).await;
    }

    if let Some(delay_ms) = test_delay_ms {
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms.min(1000))).await;
    }

    // 添付ファイルの検証 (外部ストレージへの HeadObject 検証はトランザクション外で実行)
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
                if supplied_id.is_some() && att.message_id.as_deref() == Some(id.as_str()) {
                    if let Some(msg_res) = check_idempotent_existing_message(
                        &mut db,
                        &id,
                        &auth_user.id,
                        &payload.channel_id,
                        &payload.content,
                        payload.parent_id.as_deref(),
                        &payload.kind,
                        payload.broadcast,
                        payload.metadata.as_ref(),
                        &attachment_ids,
                    )
                    .await?
                    {
                        let mut resp_headers = HeaderMap::new();
                        if state.config.dev_auth_enabled {
                            resp_headers.insert(
                                "X-Test-Fallback",
                                "attachment-already-attached".parse().unwrap(),
                            );
                        }
                        return Ok((resp_headers, Json(ApiResponse::new(msg_res, None))));
                    }
                }
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

            let metadata = crate::storage::verify::uploaded_image_metadata(
                &state.s3_client,
                &state.config.s3.bucket,
                &att.storage_key,
                att.file_size,
                &att.content_type,
            )
            .await?;
            verified_attachments.push((att.id, metadata));
        }
    }

    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let options = serde_json::json!({"kind":payload.kind,"broadcast":payload.broadcast,"metadata":payload.metadata});
    crate::apps::one(
        &mut tx,
        "SELECT json_build_object('options',set_config('zoo.message_options',$1,true))::text",
        &[&options.to_string()],
    )
    .await?;

    let principal = crate::apps::one(&mut tx,
        "SELECT json_build_object('bot',EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_bot),'active',EXISTS(SELECT 1 FROM apps a JOIN app_installations i ON i.app_id=a.id WHERE a.bot_user_id=$1 AND a.status='installed' AND i.active))::text", &[auth_user.id.as_str()]).await?;
    let bot_actor = principal["bot"] == true;
    if bot_actor && principal["active"] != true {
        return Err(AppError::Forbidden("App is not installed".into()));
    }

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

    // プライベートチャンネルの場合、所属していなければ 404 で存在を隠蔽（アーカイブ確認より前に検証）
    if channel.is_private || bot_actor || crate::apps::events::guest(&mut tx, &auth_user.id).await?
    {
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

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot post message to an archived channel".into(),
        ));
    }

    crate::apps::events::posting(
        &mut tx,
        &auth_user.id,
        &payload.channel_id,
        state.config.dev_auth_enabled,
    )
    .await?;
    // チャンネルの行を FOR SHARE で読み、membership_epoch を増やす処理と直列にしてから epoch を比べる。
    if let Some(sender) = &sender {
        let current = e2ee_channels::lock_channel_epoch(&mut tx, &payload.channel_id).await?;
        if !current.encrypted {
            return Err(e2ee_channels::plaintext_forbidden());
        }
        // ロックより前の所属の確認のあとに退出、キックが確定していたら、ここで拒否する。
        e2ee_channels::require_member(
            &mut tx,
            &payload.channel_id,
            &auth_user.id,
            &format!("Channel not found: {}", payload.channel_id),
        )
        .await?;
        e2ee_channels::check_epoch(payload.epoch.unwrap_or(0), current.membership_epoch)?;
        e2ee_channels::check_sender_device(&mut tx, &auth_user.id, sender).await?;
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

        parent_author_id = Some(parent.user_id);
    }

    let now = crate::models::now_utc_millis();

    // 投稿者は必ず認証済みユーザー (auth_user.id) を強制
    let create_result = if sender.is_some() {
        // 暗号文の列は toasty のモデルにないので、SQL で入れる。
        let ciphertext = payload
            .ciphertext
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let inserted = toasty::sql::statement(
            "INSERT INTO messages (id, user_id, channel_id, content, card, created_at, parent_id, ciphertext, epoch, revision)
             VALUES ($1, $2, $3, '', NULLIF($4, ''), $5::timestamp, NULLIF($6, ''), $7::jsonb, $8::bigint, 0)",
        )
        .bind(id.as_str())
        .bind(auth_user.id.as_str())
        .bind(payload.channel_id.as_str())
        .bind(card.as_deref().unwrap_or_default())
        .bind(now.to_string())
        .bind(payload.parent_id.as_deref().unwrap_or_default())
        .bind(ciphertext.as_str())
        .bind(payload.epoch.unwrap_or(0).to_string())
        .exec(&mut tx)
        .await;
        match inserted {
            Ok(_) => Message::get_by_id(&mut tx, id.as_str()).await,
            Err(err) => Err(err),
        }
    } else {
        toasty::create!(Message {
            id: id.as_str(),
            user_id: auth_user.id.as_str(),
            channel_id: payload.channel_id.as_str(),
            content: payload.content.as_str(),
            card,
            apub: false,
            apub_visibility: None,
            created_at: now,
            updated_at: None,
            deleted_at: None,
            parent_id: payload.parent_id.clone(),
        })
        .exec(&mut tx)
        .await
    };

    let message = match create_result {
        Ok(msg) => msg,
        Err(err) if supplied_id.is_some() && AppError::is_unique_violation(&err, None) => {
            let _ = tx.rollback().await;
            let mut retry_db = state.db.clone();
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
                if let Some(msg_res) = check_idempotent_existing_message(
                    &mut retry_db,
                    &id,
                    &auth_user.id,
                    &payload.channel_id,
                    &payload.content,
                    payload.parent_id.as_deref(),
                    &payload.kind,
                    payload.broadcast,
                    payload.metadata.as_ref(),
                    &attachment_ids,
                )
                .await?
                {
                    return Ok((HeaderMap::new(), Json(ApiResponse::new(msg_res, None))));
                }
            }
            return Err(AppError::Conflict("Idempotency key already used".into()));
        }
        Err(err) => return Err(AppError::from(err)),
    };

    // 添付ファイルの紐付け (条件付き原子的 UPDATE)
    for (att_id, metadata) in &verified_attachments {
        let rows = toasty::sql::query(
            "UPDATE attachments SET message_id = $1, width = NULLIF($5,'')::integer, height = NULLIF($6,'')::integer, blurhash = NULLIF($7,''), is_sensitive = is_sensitive OR $8::boolean OR (SELECT is_nsfw FROM channels WHERE id=$4)
             WHERE id = $2 AND uploader_id = $3 AND channel_id = $4
               AND message_id IS NULL
               AND created_at > (now() AT TIME ZONE 'UTC') - INTERVAL '23 hours'
             RETURNING id",
        )
        .bind(&id)
        .bind(att_id)
        .bind(&auth_user.id)
        .bind(&payload.channel_id)
        .bind(metadata.as_ref().map(|m| m.width.to_string()).unwrap_or_default())
        .bind(metadata.as_ref().map(|m| m.height.to_string()).unwrap_or_default())
        .bind(metadata.as_ref().map(|m| m.blurhash.as_str()).unwrap_or_default())
        .bind((payload.sensitive || payload.sensitive_attachment_ids.contains(att_id)).to_string())
        .exec(&mut tx)
        .await?;

        if rows.len() != 1 {
            let _ = tx.rollback().await;
            return Err(AppError::BadRequest(format!(
                "Failed to attach file {att_id}: conflict, already attached, or expired"
            )));
        }
    }

    let notifications = if notify {
        crate::notifications::create_message_notifications(
            &mut tx,
            &message,
            auth_user.id.as_str(),
            parent_author_id.as_deref(),
        )
        .await?
    } else {
        Vec::new()
    };

    let txid = get_current_txid(&mut tx).await?;
    if let Err(err) = tx.commit().await {
        if supplied_id.is_some() && AppError::is_unique_violation(&err, None) {
            let mut retry_db = state.db.clone();
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
                if let Some(msg_res) = check_idempotent_existing_message(
                    &mut retry_db,
                    &id,
                    &auth_user.id,
                    &payload.channel_id,
                    &payload.content,
                    payload.parent_id.as_deref(),
                    &payload.kind,
                    payload.broadcast,
                    payload.metadata.as_ref(),
                    &attachment_ids,
                )
                .await?
                {
                    return Ok((HeaderMap::new(), Json(ApiResponse::new(msg_res, None))));
                }
            }
            return Err(AppError::Conflict("Idempotency key already used".into()));
        }
        return Err(AppError::from(err));
    }
    if let Some(gate_key) = test_gate_notify {
        let gate = get_or_create_test_gate(&gate_key);
        gate.notify_waiters();
    }
    crate::push::dispatch(state.clone(), notifications);

    let mut response = MessageResponse::from(message);
    if sender.is_some() {
        response.ciphertext = payload.ciphertext.clone();
        response.epoch = payload.epoch;
    }
    Ok((HeaderMap::new(), Json(ApiResponse::new(response, txid))))
}

pub(crate) async fn update_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMessageRequest>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    validate_message_content_with_options(&payload.content, true)?;

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

    if channel.is_private || crate::apps::events::guest(&mut tx, &auth_user.id).await? {
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

    // ロックの順：チャンネルの行（FOR SHARE。membership_epoch の増加と直列にする）、メッセージの行（FOR UPDATE）。
    let current = e2ee_channels::lock_channel_epoch(&mut tx, &message.channel_id).await?;
    if current.encrypted {
        // ロックより前の所属の確認のあとに退出、キックが確定していたら、ここで拒否する。
        e2ee_channels::require_member(
            &mut tx,
            &message.channel_id,
            &auth_user.id,
            &format!("Message not found: {id}"),
        )
        .await?;
    }
    let row = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('revision', revision, 'deleted', deleted_at IS NOT NULL)::text
         FROM messages WHERE id = $1 FOR UPDATE",
        &[id.as_str()],
    )
    .await?
    .into_iter()
    .next()
    .filter(|row| row["deleted"] != true)
    .ok_or_else(|| AppError::NotFound(format!("Message not found: {id}")))?;
    let current_revision = row["revision"].as_i64().unwrap_or(0);
    let now = crate::models::now_utc_millis();

    if current.encrypted {
        if !payload.content.is_empty() {
            return Err(e2ee_channels::plaintext_forbidden());
        }
        let Some(ciphertext) = payload.ciphertext.as_ref().filter(|v| !v.is_null()) else {
            return Err(e2ee_channels::ciphertext_required());
        };
        let (Some(epoch), Some(revision)) = (payload.epoch, payload.revision) else {
            return Err(AppError::BadRequest(
                "epoch and revision are required in an encrypted channel".into(),
            ));
        };
        let sender = e2ee_channels::check_ciphertext_shape(ciphertext)?;
        e2ee_channels::check_epoch(epoch, current.membership_epoch)?;
        if revision != current_revision + 1 {
            return Err(e2ee_channels::stale_revision(current_revision));
        }
        e2ee_channels::check_sender_device(&mut tx, &auth_user.id, &sender).await?;
        toasty::sql::statement(
            "UPDATE messages SET ciphertext = $2::jsonb, epoch = $3::bigint, revision = $4::integer,
               updated_at = $5::timestamp
             WHERE id = $1",
        )
        .bind(id.as_str())
        .bind(ciphertext.to_string())
        .bind(epoch.to_string())
        .bind(revision.to_string())
        .bind(now.to_string())
        .exec(&mut tx)
        .await?;
    } else {
        if payload.ciphertext.is_some() || payload.epoch.is_some() {
            return Err(e2ee_channels::plaintext_forbidden());
        }
        // 省略したとき（更新前のクライアント）は照合せず、1 増やすだけ。
        if let Some(revision) = payload.revision
            && revision != current_revision + 1
        {
            return Err(e2ee_channels::stale_revision(current_revision));
        }
        if payload.content.trim().is_empty() {
            let has_attachments =
                Attachment::filter(Attachment::fields().message_id().eq(id.as_str()))
                    .first()
                    .exec(&mut tx)
                    .await?
                    .is_some();
            validate_message_content_with_options(&payload.content, has_attachments)?;
        }
        toasty::sql::statement(
            "UPDATE messages SET content = $2, updated_at = $3::timestamp, revision = revision + 1
             WHERE id = $1",
        )
        .bind(id.as_str())
        .bind(payload.content.as_str())
        .bind(now.to_string())
        .exec(&mut tx)
        .await?;
    }

    let updated_message = Message::get_by_id(&mut tx, id.as_str()).await?;
    let response = with_e2ee_fields(&mut tx, MessageResponse::from(updated_message)).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(response, txid)))
}

pub(crate) async fn delete_message(
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

    if channel.is_private || crate::apps::events::guest(&mut tx, &auth_user.id).await? {
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
    // 暗号文は DB のトリガーが消している。
    let response = with_e2ee_fields(&mut tx, MessageResponse::from(deleted_message)).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(response, txid)))
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

    if channel.is_private || crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
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

    let mut res: Vec<MessageResponse> = messages.into_iter().map(MessageResponse::from).collect();
    fill_e2ee_fields(&mut db, &mut res).await?;
    Ok(Json(res))
}

pub(crate) async fn add_reaction(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<AddReactionRequest>,
) -> Result<Json<ApiResponse<ReactionResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let normalized_emoji = reaction_emoji(&state, &id, &payload.emoji, true).await?;

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

    if channel.is_private || crate::apps::events::guest(&mut tx, &auth_user.id).await? {
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

    let mut notifications = Vec::new();
    let reaction = match created_res {
        Ok(r) => {
            if let Some(notification) = crate::notifications::create_reaction_notification(
                &mut tx,
                &message,
                auth_user.id.as_str(),
            )
            .await?
            {
                notifications.push(notification);
            }
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

    crate::push::dispatch(state.clone(), notifications);
    Ok(Json(ApiResponse::new(
        ReactionResponse::from(reaction),
        txid,
    )))
}

pub(crate) async fn delete_reaction(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DeleteReactionQuery>,
) -> Result<Json<ApiResponse<ReactionResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("message id is not a valid UUID: {id}")))?;

    let normalized_emoji = reaction_emoji(&state, &id, &query.emoji, false).await?;

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

    if channel.is_private || crate::apps::events::guest(&mut tx, &auth_user.id).await? {
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
                    .eq(normalized_emoji.as_str())
                    .or(MessageReaction::fields().emoji().eq(query.emoji.trim())),
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

async fn reaction_emoji(
    state: &AppState,
    message_id: &str,
    input: &str,
    require_existing: bool,
) -> Result<String, AppError> {
    if crate::validation::account_shortcode_username(input).is_some() {
        return normalize_and_validate_emoji(input);
    }
    if let Some(name) = input
        .trim()
        .strip_prefix(':')
        .and_then(|s| s.strip_suffix(':'))
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        if let Some(emoji) = crate::apps::resources::resolve_workspace_emoji(&mut tx, name).await? {
            // Keep a message's existing spelling across registry renames so old chips,
            // aliases and the picker all join the same reaction group.
            let canonical = format!(":{}:", emoji["name"].as_str().unwrap());
            let mut spellings = vec![canonical.clone()];
            if let Some(aliases) = emoji["aliases"].as_array() {
                spellings.extend(
                    aliases
                        .iter()
                        .filter_map(|a| a.as_str())
                        .map(|a| format!(":{a}:")),
                );
            }
            let rows = crate::apps::query(&mut tx, "SELECT to_json(emoji)::text FROM message_reactions WHERE message_id=$1 AND emoji IN (SELECT jsonb_array_elements_text($2::jsonb)) ORDER BY created_at,id LIMIT 1", &[message_id, &serde_json::json!(spellings).to_string()]).await?;
            if let Some(name) = rows.first().and_then(|row| row.as_str()) {
                return Ok(name.to_owned());
            }
            return Ok(if require_existing {
                canonical
            } else {
                input.trim().to_owned()
            });
        }
        if !require_existing {
            return normalize_and_validate_emoji(input);
        }
    }
    let normalized = normalize_and_validate_emoji(input)?;
    if normalized.starts_with(':') {
        return Err(AppError::BadRequest("Unknown custom emoji".into()));
    }
    Ok(normalized)
}
