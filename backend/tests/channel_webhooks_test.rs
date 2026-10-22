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
async fn delivery(
    app: &Router,
    path: &str,
    secret: &str,
    event: &str,
    delivery_id: &str,
) -> StatusCode {
    let body =
        json!({"ref":"refs/heads/main", "commits":[], "repository":{"full_name":"test/repo"}})
            .to_string();
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
                .header("x-hub-signature-256", format!("sha256={sig}"))
                .header("x-github-event", event)
                .header("x-github-delivery", delivery_id)
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
        json!({"name":"GitHub notifications"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(no_store);
    let hook = &created["data"];
    let id = hook["id"].as_str().unwrap();
    let secret = hook["secret"].as_str().unwrap();
    let receive = hook["path"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    let (_, list, no_store) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert!(no_store);
    assert_eq!(list["data"][0]["secret"], secret);
    assert_eq!(
        delivery(&app, receive, "wrong", "ping", &Uuid::new_v4().to_string()).await,
        StatusCode::UNAUTHORIZED
    );
    let delivery_id = Uuid::new_v4().to_string();
    assert_eq!(
        delivery(&app, receive, secret, "push", &delivery_id).await,
        StatusCode::OK
    );
    assert_eq!(
        delivery(&app, receive, secret, "push", &delivery_id).await,
        StatusCode::OK
    );
    let messages = Message::filter(Message::fields().channel_id().eq(channel.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(messages.len(), 1);
    assert_ne!(messages[0].user_id, owner);
    assert!(messages[0].content.starts_with("zoo:github:v1\n"));
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
        delivery(&app, receive, secret, "ping", &delivery_id).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        delivery(&app, receive, next, "ping", &delivery_id).await,
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
        delivery(&app, receive, next, "push", &Uuid::new_v4().to_string()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "DELETE", &item, &owner, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        delivery(&app, receive, next, "ping", &delivery_id).await,
        StatusCode::NOT_FOUND
    );
    let (_, list, _) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    Ok(())
}
