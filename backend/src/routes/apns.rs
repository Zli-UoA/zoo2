use crate::{
    apns::{ApnsConfig, digest, validate_registration},
    auth::AuthUser,
    error::AppError,
    models::{ApnsRevocation, ApnsSubscription, now_utc_millis},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/push/apns/config", get(config))
        .route("/api/push/apns/subscriptions", post(subscribe))
        .route("/api/push/apns/revoke", post(revoke))
}

async fn config(_actor: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "sandbox": ApnsConfig::from_env("sandbox").map_err(AppError::Internal)?.is_some(),
        "production": ApnsConfig::from_env("production").map_err(AppError::Internal)?.is_some(),
    })))
}

#[derive(Deserialize)]
struct Registration {
    device_token: String,
    environment: String,
    revocation_secret: String,
    created_at: i64,
}

async fn subscribe(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(body): Json<Registration>,
) -> Result<Json<Value>, AppError> {
    validate_registration(
        &body.device_token,
        &body.environment,
        &body.revocation_secret,
    )?;
    if ApnsConfig::from_env(&body.environment)
        .map_err(AppError::Internal)?
        .is_none()
    {
        return Err(AppError::BadRequest("Native push is not configured".into()));
    }
    let id = digest(&format!("{}:{}", body.environment, body.device_token));
    let secret_hash = digest(&body.revocation_secret);
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, &id).await?;
    validate_registration_time(body.created_at, unix_seconds()?)?;
    let revocation_id = digest(&format!("{id}:{secret_hash}"));
    if ApnsRevocation::filter(ApnsRevocation::fields().id().eq(revocation_id.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "This registration capability has been revoked".into(),
        ));
    }
    let existing = ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut tx)
        .await?;
    if let Some(existing) = existing {
        // A lost response can retry exactly the same registration; another account cannot take it over.
        if existing.user_id != actor.id || existing.revocation_hash != secret_hash {
            return Err(AppError::Conflict(
                "Revoke the previous device registration before subscribing".into(),
            ));
        }
    } else {
        let devices =
            ApnsSubscription::filter(ApnsSubscription::fields().user_id().eq(actor.id.as_str()))
                .exec(&mut tx)
                .await?;
        if devices.len() >= 20 {
            return Err(AppError::BadRequest("Too many subscribed devices".into()));
        }
        ApnsSubscription::create()
            .id(id.as_str())
            .user_id(actor.id)
            .device_token(body.device_token)
            .environment(body.environment)
            .revocation_hash(secret_hash)
            .created_at(now_utc_millis())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"id": id})))
}

#[derive(Deserialize)]
struct Revocation {
    id: String,
    revocation_secret: String,
}

// A narrowly scoped capability permits cleanup even after the access token expires.
async fn revoke(
    State(state): State<AppState>,
    Json(body): Json<Revocation>,
) -> Result<Json<Value>, AppError> {
    limit_revocations()?;
    if !crate::apns::is_hex(&body.id, 64, 64)
        || !crate::apns::is_hex(&body.revocation_secret, 64, 64)
    {
        return Err(AppError::BadRequest("Invalid revocation capability".into()));
    }
    let mut db = state.db.clone();
    let secret_hash = digest(&body.revocation_secret);
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, "apns-revocations-capacity").await?;
    lock_registration(&mut tx, &body.id).await?;
    toasty::sql::statement(
        "DELETE FROM apns_revocations WHERE revoked_at < clock_timestamp() - interval '11 minutes'",
    )
    .exec(&mut tx)
    .await?;
    let revocation_id = digest(&format!("{}:{secret_hash}", body.id));
    let inserted = toasty::sql::query(
        "INSERT INTO apns_revocations (id, revoked_at)
         SELECT $1, clock_timestamp() WHERE (SELECT count(*) FROM apns_revocations) < 10000
         ON CONFLICT (id) DO UPDATE SET revoked_at = EXCLUDED.revoked_at RETURNING id",
    )
    .bind(revocation_id)
    .exec(&mut tx)
    .await?;
    if inserted.is_empty() {
        return Err(AppError::TooManyRequests(
            "Push cleanup is busy; retry later".into(),
        ));
    }
    ApnsSubscription::filter(
        ApnsSubscription::fields().id().eq(body.id.as_str()).and(
            ApnsSubscription::fields()
                .revocation_hash()
                .eq(secret_hash.as_str()),
        ),
    )
    .delete()
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

async fn lock_registration(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<(), AppError> {
    // Both endpoints share a transaction lock, including when the subscription row does not exist yet.
    toasty::sql::query("SELECT 1::text FROM pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(id)
        .exec(tx)
        .await?;
    Ok(())
}

fn unix_seconds() -> Result<i64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs() as i64)
        .map_err(|error| AppError::Internal(error.into()))
}

fn validate_registration_time(created: i64, now: i64) -> Result<(), AppError> {
    if created < now.saturating_sub(600) || created > now.saturating_add(30) {
        return Err(AppError::BadRequest("Push registration expired or device time is incorrect; check automatic date/time and retry".into()));
    }
    Ok(())
}

fn limit_revocations() -> Result<(), AppError> {
    use std::{
        collections::VecDeque,
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    static REQUESTS: OnceLock<Mutex<VecDeque<Instant>>> = OnceLock::new();
    let mut requests = REQUESTS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("Push rate limiter unavailable")))?;
    let now = Instant::now();
    while requests
        .front()
        .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
    {
        requests.pop_front();
    }
    if requests.len() >= 120 {
        return Err(AppError::TooManyRequests(
            "Push cleanup rate limit reached; retry later".into(),
        ));
    }
    requests.push_back(now);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_deadline_bounds_delayed_requests_and_clock_skew() {
        assert!(validate_registration_time(1000, 1000).is_ok());
        assert!(validate_registration_time(400, 1000).is_ok());
        assert!(validate_registration_time(1030, 1000).is_ok());
        for created in [399, 1031, i64::MIN, i64::MAX] {
            assert!(validate_registration_time(created, 1000).is_err());
        }
    }
}
