use super::{
    response::ApiResponse,
    webhooks::{Settings, persist_card, verify_token, webhook_token},
};
use crate::{
    auth::{
        AuthUser,
        rbac::{can_manage_channel, is_workspace_admin},
    },
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, ChannelWebhook, User},
    state::AppState,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    middleware,
    response::Response,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
pub struct WebhookResponse {
    id: String,
    name: String,
    bot_user_id: String,
    path: String,
    created_at: String,
}
impl From<ChannelWebhook> for WebhookResponse {
    fn from(w: ChannelWebhook) -> Self {
        Self {
            path: format!("/api/webhooks/{}?token={}", w.id, webhook_token(&w.secret)),
            id: w.id,
            name: w.name,
            bot_user_id: w.user_id.clone(),
            created_at: w.created_at.to_string(),
        }
    }
}
#[derive(Deserialize)]
pub struct CreateWebhook {
    name: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/channels/{channel_id}/webhooks",
            get(list).post(create),
        )
        .route("/api/channels/{channel_id}/webhooks/{id}", delete(remove))
        .route(
            "/api/channels/{channel_id}/webhooks/{id}/app",
            post(associate_app),
        )
        .route(
            "/api/channels/{channel_id}/webhooks/{id}/rotate-secret",
            post(rotate),
        )
        .route("/api/webhooks/{id}", post(receive))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    actor: &AuthUser,
    id: &str,
) -> Result<Channel, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    let channel = Channel::filter(Channel::fields().id().eq(id))
        .first()
        .exec(&mut *tx)
        .await?
        .filter(|c| c.deleted_at.is_none() && !c.is_dm)
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    let admin = is_workspace_admin(tx, &actor.id, state.config.dev_auth_enabled).await?;
    if channel.is_private && !admin && channel.created_user_id != actor.id {
        let member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id)
                .and(ChannelUser::fields().user_id().eq(actor.id.as_str())),
        )
        .first()
        .exec(&mut *tx)
        .await?;
        if member.is_none() {
            return Err(AppError::NotFound("Channel not found".into()));
        }
    }
    if !can_manage_channel(&channel.created_user_id, &actor.id, admin, channel.is_dm) {
        return Err(AppError::Forbidden(
            "Only channel managers can manage webhooks".into(),
        ));
    }
    Ok(channel)
}

async fn find(
    tx: &mut toasty::Transaction<'_>,
    channel: &str,
    id: &str,
) -> Result<ChannelWebhook, AppError> {
    ChannelWebhook::filter(
        ChannelWebhook::fields()
            .id()
            .eq(id)
            .and(ChannelWebhook::fields().channel_id().eq(channel)),
    )
    .first()
    .exec(tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Webhook not found".into()))
}

fn new_secret() -> String {
    // Two independent random v4 UUIDs provide 244 random bits.
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel): Path<String>,
) -> Result<Json<ApiResponse<Vec<WebhookResponse>>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &state, &actor, &channel).await?;
    let hooks = ChannelWebhook::filter(ChannelWebhook::fields().channel_id().eq(channel.as_str()))
        .exec(&mut tx)
        .await?;
    Ok(Json(ApiResponse::new(
        hooks.into_iter().map(Into::into).collect(),
        None,
    )))
}

async fn create(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel): Path<String>,
    Json(payload): Json<CreateWebhook>,
) -> Result<Json<ApiResponse<WebhookResponse>>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::BadRequest(
            "Webhook name must be between 1 and 80 characters".into(),
        ));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let target = authorize(&mut tx, &state, &actor, &channel).await?;
    if target.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot create a webhook in an archived channel".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let username = format!("webhook_{}", Uuid::new_v4().simple());
    let secret = new_secret();
    let now = crate::models::now_utc_millis();
    toasty::create!(User {
        is_bot: true,
        id: user.as_str(),
        user_name: username.as_str(),
        display_name: name,
        created_at: now,
        updated_at: None
    })
    .exec(&mut tx)
    .await?;
    crate::apps::management::attach_webhook(&mut tx, &user, &actor.id, name).await?;
    toasty::create!(ChannelUser {
        channel_id: channel.as_str(),
        user_id: user.as_str(),
        joined_at: now
    })
    .exec(&mut tx)
    .await?;
    let hook = toasty::create!(ChannelWebhook {
        id: id.as_str(),
        channel_id: channel.as_str(),
        name,
        secret: secret.as_str(),
        user_id: user.as_str(),
        created_at: now
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(hook.into(), txid)))
}

async fn remove(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel, id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &state, &actor, &channel).await?;
    let hook = find(&mut tx, &channel, &id).await?;
    ChannelWebhook::filter(ChannelWebhook::fields().id().eq(id.as_str()))
        .delete()
        .exec(&mut tx)
        .await?;
    remove_unused_membership(&mut tx, &channel, &hook.user_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(id, txid)))
}

async fn rotate(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel, id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<WebhookResponse>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &state, &actor, &channel).await?;
    find(&mut tx, &channel, &id).await?;
    let secret = new_secret();
    ChannelWebhook::update_by_id(id.as_str())
        .secret(secret.as_str())
        .exec(&mut tx)
        .await?;
    let hook = find(&mut tx, &channel, &id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(hook.into(), txid)))
}

#[derive(Deserialize)]
struct WebhookAuthentication {
    #[serde(default)]
    token: String,
}

async fn receive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(auth): Query<WebhookAuthentication>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<&'static str, AppError> {
    let mut db = state.db.clone();
    let hook = ChannelWebhook::filter(ChannelWebhook::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut db)
        .await?
        .ok_or_else(|| AppError::NotFound("Webhook not found".into()))?;
    // Flux can keep the URL free of credentials and redact Secret.token on errors.
    // A supplied Authorization header takes precedence, including when invalid.
    let token = if let Some(value) = headers.get(axum::http::header::AUTHORIZATION) {
        value
            .to_str()
            .ok()
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
            .map(|(_, token)| token)
            .ok_or_else(|| AppError::Unauthorized("Invalid webhook token".into()))?
    } else {
        &auth.token
    };
    if !verify_token(&hook.secret, token) {
        return Err(AppError::Unauthorized("Invalid webhook token".into()));
    }
    let cards = crate::cards::parse_webhook_cards(&body)?;
    let generated_delivery = Uuid::new_v4().to_string();
    let delivery = headers
        .get("x-webhook-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(&generated_delivery);
    persist_card(
        state,
        Settings {
            channel: hook.channel_id,
            user: hook.user_id,
        },
        cards,
        delivery,
    )
    .await?;
    Ok("ok")
}

#[derive(Deserialize)]
struct AssociateApp {
    app_id: String,
}
async fn remove_unused_membership(
    tx: &mut toasty::Transaction<'_>,
    channel: &str,
    user: &str,
) -> Result<(), AppError> {
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2 AND NOT EXISTS(SELECT 1 FROM channel_webhooks WHERE channel_id=$1 AND user_id=$2)").bind(channel).bind(user).exec(tx).await?;
    toasty::sql::statement("UPDATE apps SET status='disabled' WHERE bot_user_id=$1 AND manifest::jsonb->>'notification_only'='true' AND NOT EXISTS(SELECT 1 FROM channel_webhooks WHERE user_id=$1)").bind(user).exec(tx).await?;
    Ok(())
}
async fn associate_app(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel, id)): Path<(String, String)>,
    Json(body): Json<AssociateApp>,
) -> Result<Json<ApiResponse<WebhookResponse>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &state, &actor, &channel).await?;
    let app = crate::apps::can_manage(&mut tx, &state, &actor, &body.app_id).await?;
    let manifest: crate::apps::Manifest = serde_json::from_str(app["manifest"].as_str().unwrap())
        .map_err(|e| AppError::Internal(e.into()))?;
    if app["status"] != "installed" || !manifest.notification_only {
        return Err(AppError::BadRequest(
            "Select an installed notification-only app".into(),
        ));
    }
    let old = find(&mut tx, &channel, &id).await?;
    let bot = app["bot_user_id"].as_str().unwrap();
    ChannelWebhook::update_by_id(id.as_str())
        .user_id(bot)
        .exec(&mut tx)
        .await?;
    toasty::sql::statement("INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) ON CONFLICT DO NOTHING").bind(channel.as_str()).bind(bot).exec(&mut tx).await?;
    remove_unused_membership(&mut tx, &channel, &old.user_id).await?;
    let hook = find(&mut tx, &channel, &id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(hook.into(), txid)))
}
