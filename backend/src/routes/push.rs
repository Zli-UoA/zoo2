use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    auth::AuthUser,
    error::AppError,
    models::{PushSubscription, now_utc_millis},
    push::PushConfig,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/push/config", get(config)).route(
        "/api/push/subscriptions",
        post(subscribe).delete(unsubscribe),
    )
}

async fn config(_actor: AuthUser) -> Result<Json<Value>, AppError> {
    let config = PushConfig::from_env().map_err(AppError::Internal)?;
    Ok(Json(json!({"publicKey": config.map(|c| c.public_key)})))
}

#[derive(Deserialize)]
struct SubscriptionRequest {
    endpoint: String,
    keys: crate::push::SubscriptionKeys,
}

#[derive(Deserialize)]
struct UnsubscribeRequest {
    endpoint: String,
}

fn endpoint_id(endpoint: &str) -> String {
    Sha256::digest(endpoint.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

async fn subscribe(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(body): Json<SubscriptionRequest>,
) -> Result<Json<Value>, AppError> {
    if PushConfig::from_env()
        .map_err(AppError::Internal)?
        .is_none()
    {
        return Err(AppError::BadRequest(
            "Push notifications are not configured".into(),
        ));
    }
    crate::push::validate_endpoint(&body.endpoint)?;
    body.keys
        .validate()
        .map_err(|_| AppError::BadRequest("Invalid push subscription keys".into()))?;
    let id = endpoint_id(&body.endpoint);
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let existing = PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut tx)
        .await?;
    if let Some(existing) = existing {
        if existing.user_id != actor.id {
            return Err(AppError::Conflict(
                "Unsubscribe this browser before changing accounts".into(),
            ));
        }
        PushSubscription::update_by_id(id.as_str())
            .p256dh(Some(body.keys.p256dh))
            .auth_secret(Some(body.keys.auth))
            .exec(&mut tx)
            .await?;
    } else {
        let subscriptions =
            PushSubscription::filter(PushSubscription::fields().user_id().eq(actor.id.as_str()))
                .exec(&mut tx)
                .await?;
        if subscriptions.len() >= 20 {
            return Err(AppError::BadRequest("Too many subscribed devices".into()));
        }
        PushSubscription::create()
            .id(id)
            .user_id(actor.id)
            .endpoint(body.endpoint)
            .p256dh(Some(body.keys.p256dh))
            .auth_secret(Some(body.keys.auth))
            .created_at(now_utc_millis())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

async fn unsubscribe(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(body): Json<UnsubscribeRequest>,
) -> Result<Json<Value>, AppError> {
    let id = endpoint_id(&body.endpoint);
    let mut db = state.db.clone();
    PushSubscription::filter(
        PushSubscription::fields()
            .id()
            .eq(id.as_str())
            .and(PushSubscription::fields().user_id().eq(actor.id.as_str())),
    )
    .delete()
    .exec(&mut db)
    .await?;
    Ok(Json(json!({"ok": true})))
}
