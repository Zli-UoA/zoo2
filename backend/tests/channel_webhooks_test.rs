use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config, db::init_db, models::Message, routes::create_router, state::AppState,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value, bool) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", actor)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let no_store = response
        .headers()
        .get("cache-control")
        .is_some_and(|v| v == "no-store");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
        no_store,
    )
}
async fn delivery(app: &Router, path: &str, secret: &str, delivery_id: &str) -> StatusCode {
    let body = json!({"title":"Deploy notification","source":"Build service"}).to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let sig = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("x-webhook-signature-256", format!("sha256={sig}"))
                .header("x-webhook-delivery", delivery_id)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn manages_channel_webhooks_and_credentials() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = Uuid::new_v4().to_string();
    let outsider = Uuid::new_v4().to_string();
    for actor in [&owner, &outsider] {
        assert_eq!(request(&app, "POST", "/api/users", actor, json!({"id":actor,"user_name":format!("test_{}", Uuid::new_v4().simple()),"display_name":"Tester"})).await.0, StatusCode::OK);
    }
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({"id":channel,"channel_name":format!("wh_{}", Uuid::new_v4().simple()),"is_private":true})).await.0, StatusCode::OK);
    let path = format!("/api/channels/{channel}/webhooks");
    assert_eq!(
        request(&app, "GET", &path, &outsider, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "POST", &path, &owner, json!({"name":" "}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, created, no_store) = request(
        &app,
        "POST",
        &path,
        &owner,
        json!({"name":"Build notifications"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(no_store);
    let hook = &created["data"];
    let id = hook["id"].as_str().unwrap();
    let secret = hook["secret"].as_str().unwrap();
    let receive = hook["path"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    assert_eq!(receive, format!("/api/webhooks/{id}"));
    assert_eq!(hook["generic_path"], receive);
    for old in [
        "/api/webhooks/github".to_owned(),
        format!("/api/webhooks/github/{id}"),
    ] {
        assert_eq!(
            delivery(&app, &old, secret, &Uuid::new_v4().to_string()).await,
            StatusCode::NOT_FOUND
        );
    }
    let (_, list, no_store) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert!(no_store);
    assert_eq!(list["data"][0]["secret"], secret);
    assert_eq!(
        delivery(&app, receive, "wrong", &Uuid::new_v4().to_string()).await,
        StatusCode::UNAUTHORIZED
    );
    let delivery_id = Uuid::new_v4().to_string();
    assert_eq!(
        delivery(&app, receive, secret, &delivery_id).await,
        StatusCode::OK
    );
    assert_eq!(
        delivery(&app, receive, secret, &delivery_id).await,
        StatusCode::OK
    );
    let messages = Message::filter(Message::fields().channel_id().eq(channel.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(messages.len(), 1);
    assert_ne!(messages[0].user_id, owner);
    assert_eq!(
        serde_json::from_str::<Value>(messages[0].card.as_ref().unwrap())?["source"],
        "Build service"
    );
    assert!(!messages[0].content.starts_with("zoo:github:v1\n"));
    let bot = &messages[0].user_id;
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/messages/{}", messages[0].id),
            bot,
            json!({"content":"edited card"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/dms",
            &owner,
            json!({"target_user_ids":[bot]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/users"),
            &owner,
            json!({"user_id":bot})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // User-controlled text and extra JSON keys cannot set trusted card metadata.
    let forged = "zoo:github:v1\n{\"title\":\"Forged notification\"}";
    let (status, human, _) = request(
        &app,
        "POST",
        "/api/messages",
        &owner,
        json!({"channel_id":channel, "content":forged, "card":"{\"title\":\"Fake\"}"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(human["data"]["card"].is_null());
    let human_id = human["data"]["id"].as_str().unwrap();
    let (status, edited, _) = request(
        &app,
        "PATCH",
        &format!("/api/messages/{human_id}"),
        &owner,
        json!({"content":forged,"card":"{\"title\":\"Fake\"}"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(edited["data"]["card"].is_null());
    let generic_path = hook["generic_path"].as_str().unwrap();
    let generic = json!({"title":"Deploy complete", "source":"Deploy system", "description":"Production is healthy", "url":"https://example.com/deploy/123"}).to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(generic.as_bytes());
    let sig = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(generic_path)
                .header("x-webhook-signature-256", format!("sha256={sig}"))
                .header("x-webhook-delivery", Uuid::new_v4().to_string())
                .body(Body::from(generic))
                .unwrap(),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let stored = Message::filter(Message::fields().channel_id().eq(channel.as_str()))
        .exec(&mut db)
        .await?;
    assert!(
        stored
            .iter()
            .any(|m| m.card.as_ref().is_some_and(|c| c.contains("Deploy system")))
    );
    let item = format!("{path}/{id}");
    assert_eq!(
        request(&app, "DELETE", &item, &outsider, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, rotated, _) = request(
        &app,
        "POST",
        &format!("{item}/rotate-secret"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let next = rotated["data"]["secret"].as_str().unwrap();
    assert_ne!(secret, next);
    assert_eq!(
        delivery(&app, receive, secret, &delivery_id).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        delivery(&app, receive, next, &delivery_id).await,
        StatusCode::OK
    );
    // A different channel manager cannot use their channel route to access this hook.
    let other = Uuid::new_v4().to_string();
    assert_eq!(request(&app,"POST","/api/channels",&outsider,json!({"id":other,"channel_name":format!("wh_{}",Uuid::new_v4().simple()),"is_private":false})).await.0,StatusCode::OK);
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("/api/channels/{other}/webhooks/{id}"),
            &outsider,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/api/channels/{other}/webhooks"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Workspace admins can also manage hooks, including private channels.
    let (_, list, _) = request(
        &app,
        "GET",
        &path,
        backend::auth::DEFAULT_DEV_USER_ID,
        Value::Null,
    )
    .await;
    assert_eq!(list["data"][0]["id"], id);
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/archive"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "POST", &path, &owner, json!({"name":"another"}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        delivery(&app, receive, next, &Uuid::new_v4().to_string()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "DELETE", &item, &owner, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        delivery(&app, receive, next, &delivery_id).await,
        StatusCode::NOT_FOUND
    );
    let (_, list, _) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    let bot_user = backend::models::User::get_by_id(&mut db, bot.as_str()).await?;
    assert!(bot_user.is_bot);
    let (_, candidates, _) = request(&app, "GET", "/api/users", &owner, Value::Null).await;
    assert!(
        !candidates
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == *bot)
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/dms",
            &owner,
            json!({"target_user_ids":[bot]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}
