use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    models::{
        Channel, ChannelNote, ChannelUser, Notification, PushSubscription, User, now_utc_millis,
    },
    routes::create_router,
    state::AppState,
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn note_push_rechecks_channel_deletion_and_current_membership() -> anyhow::Result<()> {
    let config = Config::from_env();
    let mut db = init_db(&config.database_url).await?;
    let now = now_utc_millis();
    let recipient = Uuid::new_v4().to_string();
    let actor = Uuid::new_v4().to_string();
    for user_id in [&recipient, &actor] {
        User::create()
            .id(user_id.as_str())
            .user_name(format!("push_note_{user_id}"))
            .display_name(user_id.as_str())
            .is_bot(false)
            .created_at(now)
            .exec(&mut db)
            .await?;
    }
    let channel_id = Uuid::new_v4().to_string();
    Channel::create()
        .id(channel_id.as_str())
        .channel_name(format!("push-note-{channel_id}"))
        .created_user_id(actor.as_str())
        .is_private(true)
        .is_dm(false)
        .created_at(now)
        .exec(&mut db)
        .await?;
    ChannelUser::create()
        .user_id(recipient.as_str())
        .channel_id(channel_id.as_str())
        .joined_at(now)
        .exec(&mut db)
        .await?;
    let note_id = Uuid::new_v4().to_string();
    ChannelNote::create()
        .id(note_id.as_str())
        .channel_id(channel_id.as_str())
        .title("Private note")
        .content("private body")
        .position(0)
        .deleted_at(None)
        .version(1)
        .created_user_id(actor.as_str())
        .created_at(now)
        .updated_at(now)
        .exec(&mut db)
        .await?;
    let notification = Notification {
        id: Uuid::new_v4().to_string(),
        user_id: recipient.clone(),
        actor_id: actor,
        channel_id: channel_id.clone(),
        message_id: None,
        note_id: Some(note_id),
        notification_type: "mention".into(),
        is_read: false,
        created_at: now,
    };
    assert!(backend::push::note_push_is_currently_deliverable(&mut db, &notification).await?);
    ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id.as_str())
            .and(ChannelUser::fields().user_id().eq(recipient.as_str())),
    )
    .delete()
    .exec(&mut db)
    .await?;
    assert!(!backend::push::note_push_is_currently_deliverable(&mut db, &notification).await?);
    ChannelUser::create()
        .user_id(recipient.as_str())
        .channel_id(channel_id.as_str())
        .joined_at(now_utc_millis())
        .exec(&mut db)
        .await?;
    toasty::sql::query("UPDATE channels SET deleted_at = statement_timestamp() WHERE id = $1")
        .bind(channel_id.as_str())
        .exec(&mut db)
        .await?;
    assert!(!backend::push::note_push_is_currently_deliverable(&mut db, &notification).await?);
    Ok(())
}

#[tokio::test]
async fn push_routes_require_auth_and_unsubscribe_only_the_owner() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let mut secured = config.clone();
    secured.dev_auth_enabled = false;
    let secured_app = create_router(AppState::new(db.clone(), secured));
    let app = create_router(AppState::new(db.clone(), config));
    let alice = Uuid::new_v4().to_string();
    let bob = Uuid::new_v4().to_string();
    for user in [&alice, &bob] {
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(json!({"id":user,"user_name":format!("push_{user}"),"display_name":"Push Test"}).to_string()))?).await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    for (method, path) in [
        ("GET", "/api/push/config"),
        ("POST", "/api/push/subscriptions"),
        ("DELETE", "/api/push/subscriptions"),
    ] {
        let response = secured_app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"endpoint":"https://fcm.googleapis.com/test"}).to_string(),
                    ))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let endpoint = format!("https://fcm.googleapis.com/test/{}", Uuid::new_v4());
    use sha2::{Digest, Sha256};
    let id = Sha256::digest(endpoint.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    PushSubscription::create()
        .id(id.as_str())
        .user_id(alice.as_str())
        .endpoint(endpoint.as_str())
        .created_at(now_utc_millis())
        .exec(&mut db)
        .await?;
    let request = |user: &str| {
        Request::builder()
            .method("DELETE")
            .uri("/api/push/subscriptions")
            .header("X-Dev-User", user)
            .header("content-type", "application/json")
            .body(Body::from(json!({"endpoint":endpoint}).to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(request(&bob)).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
            .first()
            .exec(&mut db)
            .await?
            .is_some()
    );
    for _ in 0..2 {
        let response = app.clone().oneshot(request(&alice)).await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            PushSubscription::filter(PushSubscription::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?
                .is_none()
        );
    }
    Ok(())
}

#[tokio::test]
async fn registration_respects_server_configuration_and_endpoint_ownership() -> anyhow::Result<()> {
    use axum::body::to_bytes;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let users = [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()];
    for user in &users {
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(json!({"id":user,"user_name":format!("push_{user}"),"display_name":"Push Test"}).to_string()))?).await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let configured = backend::push::PushConfig::from_env()?.is_some();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/push/config")
                .header("X-Dev-User", &users[0])
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    assert_eq!(body["publicKey"].is_string(), configured);
    let endpoint = format!("https://fcm.googleapis.com/test/{}", Uuid::new_v4());
    let (receiver, auth) = ece::generate_keypair_and_auth_secret()?;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let keys = json!({"p256dh": URL_SAFE_NO_PAD.encode(receiver.pub_as_raw()?), "auth": URL_SAFE_NO_PAD.encode(auth)});
    let request = |user: &str, endpoint: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/push/subscriptions")
            .header("X-Dev-User", user)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"endpoint":endpoint,"keys":keys}).to_string(),
            ))
            .unwrap()
    };
    let invalid = app
        .clone()
        .oneshot(request(&users[0], "https://127.0.0.1/private"))
        .await?;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    for _ in 0..2 {
        let response = app.clone().oneshot(request(&users[0], &endpoint)).await?;
        assert_eq!(
            response.status(),
            if configured {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            }
        );
        if configured {
            let saved = PushSubscription::filter(
                PushSubscription::fields().endpoint().eq(endpoint.as_str()),
            )
            .first()
            .exec(&mut db)
            .await?
            .unwrap();
            assert_eq!(saved.p256dh.as_deref(), keys["p256dh"].as_str());
            assert_eq!(saved.auth_secret.as_deref(), keys["auth"].as_str());
        }
    }
    let response = app.clone().oneshot(request(&users[1], &endpoint)).await?;
    assert_eq!(
        response.status(),
        if configured {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        }
    );
    Ok(())
}
