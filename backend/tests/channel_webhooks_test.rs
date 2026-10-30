use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config, db::init_db, models::Message, routes::create_router, state::AppState,
};
use serde_json::{Value, json};
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
async fn delivery(app: &Router, path: &str, delivery_id: &str) -> StatusCode {
    slack_delivery(
        app,
        path,
        delivery_id,
        json!({"attachments":[{"title":"Deploy notification","footer":"Build service"}]}),
    )
    .await
}

async fn slack_delivery(app: &Router, path: &str, delivery_id: &str, body: Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("x-webhook-delivery", delivery_id)
                .body(Body::from(body.to_string()))
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
    let receive = hook["path"].as_str().unwrap();
    assert!(receive.starts_with(&format!("/api/webhooks/{id}?token=")));
    assert!(hook.get("secret").is_none());
    assert!(hook.get("generic_path").is_none());
    assert!(hook.get("slack_path").is_none());
    for old in [
        "/api/webhooks/github".to_owned(),
        format!("/api/webhooks/github/{id}"),
    ] {
        assert_eq!(
            delivery(&app, &old, &Uuid::new_v4().to_string()).await,
            StatusCode::NOT_FOUND
        );
    }
    let (_, list, no_store) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert!(no_store);
    assert_eq!(list["data"][0]["path"], receive);
    assert_eq!(
        delivery(
            &app,
            &format!("/api/webhooks/{id}?token=wrong"),
            &Uuid::new_v4().to_string()
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let delivery_id = Uuid::new_v4().to_string();
    assert_eq!(delivery(&app, receive, &delivery_id).await, StatusCode::OK);
    assert_eq!(delivery(&app, receive, &delivery_id).await, StatusCode::OK);
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
    // The existing endpoint now accepts only Slack messages, with URL authentication.
    assert_eq!(
        slack_delivery(
            &app,
            receive,
            &Uuid::new_v4().to_string(),
            json!({"title":"Legacy card"})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        slack_delivery(
            &app,
            &format!("/api/webhooks/{id}"),
            &Uuid::new_v4().to_string(),
            json!({"text":"Missing token"})
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        slack_delivery(
            &app,
            &format!("/api/webhooks/{id}/slack/removed"),
            &Uuid::new_v4().to_string(),
            json!({"text":"Removed route"})
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let slack_id = Uuid::new_v4().to_string();
    let slack = json!({"text":"Flux deployment", "attachments":[
        {"title":"Ready", "text":"production", "color":"good"},
        {"title":"Failed", "text":"staging", "color":"#ff0000"}
    ]});
    for _ in 0..2 {
        assert_eq!(
            slack_delivery(&app, receive, &slack_id, slack.clone()).await,
            StatusCode::OK
        );
    }
    let messages = Message::filter(Message::fields().channel_id().eq(channel.as_str()))
        .exec(&mut db)
        .await?;
    let slack_messages = messages
        .iter()
        .filter(|m| {
            m.card
                .as_ref()
                .is_some_and(|c| c.contains("Flux deployment"))
        })
        .collect::<Vec<_>>();
    assert_eq!(slack_messages.len(), 1);
    let cards: Value = serde_json::from_str(slack_messages[0].card.as_ref().unwrap())?;
    assert_eq!(cards["cards"][1]["color"], "#2eb886");
    assert_eq!(cards["cards"][2]["color"], "#ff0000");
    assert_eq!(
        slack_messages[0].content,
        "Flux deployment\nReady\nproduction\nFailed\nstaging"
    );
    // Flux sends credentials as Secret.token, keeping retry error URLs public.
    let (endpoint, token) = receive.split_once("?token=").unwrap();
    for (url, authorization, expected) in [
        (endpoint, format!("Bearer {token}"), StatusCode::OK),
        (endpoint, "Bearer wrong".into(), StatusCode::UNAUTHORIZED),
        (receive, "Basic wrong".into(), StatusCode::UNAUTHORIZED),
        (receive, "Bearer wrong".into(), StatusCode::UNAUTHORIZED),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(url)
                    .header("authorization", authorization)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"attachments":[{"text":"Reconciliation failed","color":"danger"}]}"#,
                    ))
                    .unwrap(),
            )
            .await?;
        assert_eq!(response.status(), expected);
    }
    let max_cards = json!({"attachments": vec![json!({"title":"界".repeat(500)}); 100]});
    assert_eq!(
        slack_delivery(&app, receive, &Uuid::new_v4().to_string(), max_cards).await,
        StatusCode::OK
    );
    let stored = Message::filter(Message::fields().channel_id().eq(channel.as_str()))
        .exec(&mut db)
        .await?;
    assert!(stored.iter().any(|m| m.content == "Reconciliation failed"));
    let long = stored.iter().find(|m| m.content.starts_with("界")).unwrap();
    assert_eq!(long.content.chars().count(), 40_000);
    let full: Value = serde_json::from_str(long.card.as_ref().unwrap())?;
    assert_eq!(full["cards"].as_array().unwrap().len(), 100);
    assert_eq!(full["cards"][99]["title"], "界".repeat(500));
    assert_eq!(
        slack_delivery(
            &app,
            &format!("/api/webhooks/{id}?token=invalid"),
            &slack_id,
            slack.clone()
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        slack_delivery(
            &app,
            receive,
            &slack_id,
            json!({"attachments":[{"text":"x", "color":"invalid"}]})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // Slack clients do not need custom headers or a delivery ID.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(receive)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"No custom headers"}"#))
                .unwrap(),
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
        b"ok"
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
    let next = rotated["data"]["path"].as_str().unwrap();
    assert_ne!(receive, next);
    for (bearer, expected) in [
        (token, StatusCode::UNAUTHORIZED),
        (next.split_once("?token=").unwrap().1, StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(endpoint)
                    .header("authorization", format!("Bearer {bearer}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"Rotated header token"}"#))
                    .unwrap(),
            )
            .await?;
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        delivery(&app, receive, &delivery_id).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(delivery(&app, next, &delivery_id).await, StatusCode::OK);
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
        delivery(&app, next, &Uuid::new_v4().to_string()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        slack_delivery(&app, next, &Uuid::new_v4().to_string(), slack.clone()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "DELETE", &item, &owner, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        delivery(&app, next, &delivery_id).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        slack_delivery(&app, next, &slack_id, slack).await,
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
