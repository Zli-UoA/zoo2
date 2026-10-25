use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    models::{ApnsSubscription, now_utc_millis},
    routes::create_router,
    state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn native_registration_enforces_ownership_and_capability_cleanup() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let mut secured = config.clone();
    secured.dev_auth_enabled = false;
    let secured_app = create_router(AppState::new(db.clone(), secured));
    let app = create_router(AppState::new(db.clone(), config));
    let users = [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
    for user in &users {
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(json!({"id":user,"user_name":format!("apns_{user}"),"display_name":"Push Test"}).to_string()))?).await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    for (method, path) in [
        ("GET", "/api/push/apns/config"),
        ("POST", "/api/push/apns/subscriptions"),
    ] {
        let response = secured_app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let token = backend::apns::digest(&Uuid::new_v4().to_string());
    let id = backend::apns::digest(&format!("sandbox:{token}"));
    let secret = "a".repeat(64);
    let configured = backend::apns::ApnsConfig::from_env("sandbox")?.is_some();
    let registration = |user: &str, capability: &str| {
        Request::builder().method("POST").uri("/api/push/apns/subscriptions")
        .header("content-type", "application/json").header("X-Dev-User", user)
        .body(Body::from(json!({"device_token":token,"environment":"sandbox","revocation_secret":capability,"created_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()}).to_string())).unwrap()
    };
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(registration(&users[0], &secret))
            .await?;
        assert_eq!(
            response.status(),
            if configured {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            }
        );
        if configured {
            let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
            assert_eq!(body["id"], id);
        }
    }
    if configured {
        for (user, capability) in [(&users[1], secret.clone()), (&users[0], "b".repeat(64))] {
            assert_eq!(
                app.clone()
                    .oneshot(registration(user, &capability))
                    .await?
                    .status(),
                StatusCode::CONFLICT
            );
        }
    } else {
        ApnsSubscription::create()
            .id(id.as_str())
            .user_id(users[0].as_str())
            .device_token(token.as_str())
            .environment("sandbox")
            .revocation_hash(backend::apns::digest(&secret))
            .created_at(now_utc_millis())
            .exec(&mut db)
            .await?;
    }
    let revoke = |capability: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/push/apns/revoke")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"id":id,"revocation_secret":capability,"created_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()}).to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        secured_app
            .clone()
            .oneshot(revoke(&"b".repeat(64)))
            .await?
            .status(),
        StatusCode::OK
    );
    assert!(
        ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
            .first()
            .exec(&mut db)
            .await?
            .is_some()
    );
    for _ in 0..2 {
        assert_eq!(
            secured_app.clone().oneshot(revoke(&secret)).await?.status(),
            StatusCode::OK
        );
        assert!(
            ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?
                .is_none()
        );
    }
    if configured {
        // A request that reaches the server after logout must not resurrect the revoked capability.
        assert_eq!(
            app.clone()
                .oneshot(registration(&users[0], &secret))
                .await?
                .status(),
            StatusCode::CONFLICT
        );
        let delayed_token = backend::apns::digest(&Uuid::new_v4().to_string());
        let delayed_id = backend::apns::digest(&format!("sandbox:{delayed_token}"));
        let early_revoke = Request::builder().method("POST").uri("/api/push/apns/revoke")
            .header("content-type", "application/json")
            .body(Body::from(json!({"id":delayed_id,"revocation_secret":secret,"created_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()}).to_string()))?;
        assert_eq!(
            secured_app.clone().oneshot(early_revoke).await?.status(),
            StatusCode::OK
        );
        let late_registration = Request::builder().method("POST").uri("/api/push/apns/subscriptions")
            .header("content-type", "application/json").header("X-Dev-User", &users[0])
            .body(Body::from(json!({"device_token":delayed_token,"environment":"sandbox","revocation_secret":secret,"created_at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()}).to_string()))?;
        assert_eq!(
            app.clone().oneshot(late_registration).await?.status(),
            StatusCode::CONFLICT
        );
    }
    if configured {
        assert_eq!(
            app.clone()
                .oneshot(registration(&users[1], &"c".repeat(64)))
                .await?
                .status(),
            StatusCode::OK
        );
        // A delayed old logout cannot revoke the new account's registration.
        secured_app.clone().oneshot(revoke(&secret)).await?;
        let current = ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
            .first()
            .exec(&mut db)
            .await?
            .unwrap();
        assert_eq!(current.user_id, users[1]);
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ios/oauth/callback?code=must-not-reflect&state=private")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let html = String::from_utf8(to_bytes(response.into_body(), 4096).await?.to_vec())?;
    assert!(!html.contains("must-not-reflect"));
    Ok(())
}
