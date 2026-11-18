use crate::{
    apns::{ApnsConfig, digest, lock_registration, validate_registration},
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
        .route("/api/push/apns/test", post(test_device))
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
        return Err(AppError::BadRequest(
            "iPhoneへの通知が設定されていません".into(),
        ));
    }
    let id = digest(&format!("{}:{}", body.environment, body.device_token));
    let secret_hash = digest(&body.revocation_secret);
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // 同時に異なる端末を追加してもアカウントごとの上限を守る。
    lock_registration(&mut tx, &format!("apns-user:{}", actor.id)).await?;
    lock_registration(&mut tx, &id).await?;
    validate_registration_time(body.created_at, unix_seconds()?)?;
    let revocation_id = digest(&format!("{id}:{secret_hash}"));
    if ApnsRevocation::filter(ApnsRevocation::fields().id().eq(revocation_id.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("この通知登録は解除済みです".into()));
    }
    let existing = ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut tx)
        .await?;
    if let Some(existing) = existing {
        // 応答を受け取れなかった再試行でも、同じアカウントと解除情報なら登録を維持する。
        if existing.user_id != actor.id || existing.revocation_hash != secret_hash {
            return Err(AppError::Conflict(
                "以前の端末登録を解除してから再登録してください".into(),
            ));
        }
        // Appleの失効通知が遅れても、端末が再度確認したトークンを残す。
        ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
            .update()
            .created_at(now_utc_millis())
            .exec(&mut tx)
            .await?;
    } else {
        let devices =
            ApnsSubscription::filter(ApnsSubscription::fields().user_id().eq(actor.id.as_str()))
                .exec(&mut tx)
                .await?;
        if devices.len() >= 20 {
            return Err(AppError::BadRequest("登録できる端末は20台までです".into()));
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

#[derive(Deserialize)]
struct TestRequest {
    #[serde(flatten)]
    device: Revocation,
    test_id: String,
}

async fn test_device(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(request): Json<TestRequest>,
) -> Result<Json<Value>, AppError> {
    if request.test_id.len() != 36 || uuid::Uuid::try_parse(&request.test_id).is_err() {
        return Err(AppError::BadRequest(
            "テスト通知のIDが正しくありません".into(),
        ));
    }
    let mut db = state.db.clone();
    let device = reserve_test(&mut db, &actor.id, &request.device).await?;
    crate::apns::test_device(&mut db, &device, &request.test_id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn reserve_test(
    db: &mut toasty::Db,
    user: &str,
    request: &Revocation,
) -> Result<ApnsSubscription, AppError> {
    let unavailable = || AppError::NotFound("この端末の通知登録を確認できません".into());
    if !crate::apns::is_hex(&request.id, 64, 64)
        || !crate::apns::is_hex(&request.revocation_secret, 64, 64)
    {
        return Err(unavailable());
    }
    let hash = digest(&request.revocation_secret);
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, &request.id).await?;
    let device = ApnsSubscription::filter(
        ApnsSubscription::fields()
            .id()
            .eq(request.id.as_str())
            .and(ApnsSubscription::fields().user_id().eq(user))
            .and(
                ApnsSubscription::fields()
                    .revocation_hash()
                    .eq(hash.as_str()),
            ),
    )
    .first()
    .exec(&mut tx)
    .await?
    .ok_or_else(unavailable)?;
    // 応答が途切れても二重配送しないよう、Appleに送る前にこの端末の送信枠を確保する。
    let claimed = toasty::sql::query(
        "UPDATE apns_subscriptions SET last_test_at=statement_timestamp()
         WHERE id=$1 AND (last_test_at IS NULL OR last_test_at <= statement_timestamp() - interval '1 minute')
         RETURNING id",
    ).bind(request.id.as_str()).exec(&mut tx).await?;
    if claimed.is_empty() {
        return Err(AppError::RateLimited);
    }
    tx.commit().await?;
    Ok(device)
}

// サインアウト後も以前の端末登録を解除できるよう、登録ごとに固有の解除情報を使う。
async fn revoke(
    State(state): State<AppState>,
    Json(body): Json<Revocation>,
) -> Result<Json<Value>, AppError> {
    if !crate::apns::is_hex(&body.id, 64, 64)
        || !crate::apns::is_hex(&body.revocation_secret, 64, 64)
    {
        return Err(AppError::BadRequest(
            "通知を解除する情報が正しくありません".into(),
        ));
    }
    let mut db = state.db.clone();
    let secret_hash = digest(&body.revocation_secret);
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, &body.id).await?;
    let revocation_id = digest(&format!("{}:{secret_hash}", body.id));
    let removed = toasty::sql::query(
        "DELETE FROM apns_subscriptions WHERE id=$1 AND revocation_hash=$2 RETURNING id",
    )
    .bind(body.id.as_str())
    .bind(secret_hash.as_str())
    .exec(&mut tx)
    .await?;
    if removed.is_empty() {
        // 正しい解除・同じ解除の再送は、未登録端末用の枠が埋まっても成功させる。
        if ApnsRevocation::filter(ApnsRevocation::fields().id().eq(revocation_id.as_str()))
            .first()
            .exec(&mut tx)
            .await?
            .is_some()
        {
            return Ok(Json(json!({"ok":true})));
        }
        limit_revocations()?;
        lock_registration(&mut tx, "apns-revocations-capacity").await?;
    }
    toasty::sql::statement("DELETE FROM apns_revocations WHERE revoked_at < statement_timestamp() - interval '11 minutes'")
        .exec(&mut tx).await?;
    if removed.is_empty()
        && !toasty::sql::query("SELECT 1::text FROM apns_revocations OFFSET 9999 LIMIT 1")
            .exec(&mut tx)
            .await?
            .is_empty()
    {
        return Err(AppError::RateLimited);
    }
    toasty::sql::statement(
        "INSERT INTO apns_revocations (id, revoked_at) VALUES($1, statement_timestamp())
         ON CONFLICT (id) DO UPDATE SET revoked_at=EXCLUDED.revoked_at",
    )
    .bind(revocation_id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

fn unix_seconds() -> Result<i64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs() as i64)
        .map_err(|error| AppError::Internal(error.into()))
}

fn validate_registration_time(created: i64, now: i64) -> Result<(), AppError> {
    if created < now.saturating_sub(600) || created > now.saturating_add(30) {
        return Err(AppError::BadRequest("通知の登録期限が過ぎたか、端末の時刻が正しくありません。日時を確認して再試行してください".into()));
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
        .map_err(|_| {
            AppError::Internal(anyhow::anyhow!(
                "通知解除のリクエスト数を確認できませんでした"
            ))
        })?;
    let now = Instant::now();
    while requests
        .front()
        .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
    {
        requests.pop_front();
    }
    if requests.len() >= 120 {
        return Err(AppError::RateLimited);
    }
    requests.push_back(now);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "マイグレーションを適用したPostgreSQLを使用"]
    async fn apns_self_test_claims_only_one_concurrent_send() -> anyhow::Result<()> {
        let mut db = crate::db::init_db(&crate::config::Config::from_env().database_url).await?;
        let user = uuid::Uuid::new_v4().to_string();
        let token = digest(&uuid::Uuid::new_v4().to_string());
        let id = digest(&format!("sandbox:{token}"));
        let secret = "e".repeat(64);
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'通知予約テスト')",
        )
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
        ApnsSubscription::create()
            .id(id.as_str())
            .user_id(user.as_str())
            .device_token(token)
            .environment("sandbox")
            .revocation_hash(digest(&secret))
            .created_at(now_utc_millis())
            .exec(&mut db)
            .await?;
        let request = Revocation {
            id,
            revocation_secret: secret,
        };
        let mut other = db.clone();
        let (first, second) = tokio::join!(
            reserve_test(&mut db, &user, &request),
            reserve_test(&mut other, &user, &request)
        );
        assert!(matches!(
            (first, second),
            (Ok(_), Err(AppError::RateLimited)) | (Err(AppError::RateLimited), Ok(_))
        ));
        Ok(())
    }

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
