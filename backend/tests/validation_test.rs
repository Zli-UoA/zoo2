use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

#[tokio::test]
async fn test_validation_and_duplicate_conflicts() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    unsafe {
        std::env::set_var("DEV_AUTH_ENABLED", "true");
    }

    let config = Config::from_env();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state);

    let user_name = format!("user_{}", uuid::Uuid::new_v4().simple());

    // 1. バリデーション違反 (予約語のユーザー名 'slackbot') -> 400 Bad Request
    let reserved_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "user_name": "slackbot",
            "display_name": "Bot",
        }))?))?;
    let resp = app.clone().oneshot(reserved_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2. 正常なユーザー作成 (前後に空白があってもトリム後 80 文字以内なら OK) -> 200 OK
    let valid_user_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "user_name": user_name,
            "display_name": "  Normal User  ",
        }))?))?;
    let resp = app.clone().oneshot(valid_user_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let u_body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let created_user_id = u_body["data"]["id"].as_str().unwrap().to_string();

    // 3. 重複ユーザー名の作成 -> 409 Conflict
    let dup_user_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "user_name": user_name,
            "display_name": "Duplicate User",
        }))?))?;
    let resp = app.clone().oneshot(dup_user_req).await?;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // 4. 不正な UUID によるユーザー取得 -> 400 Bad Request
    let invalid_uuid_user_req = Request::builder()
        .method("GET")
        .uri("/api/users/not-a-valid-uuid")
        .body(Body::empty())?;
    let resp = app.clone().oneshot(invalid_uuid_user_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. 不正な UUID によるチャンネル取得 -> 400 Bad Request
    let invalid_uuid_ch_req = Request::builder()
        .method("GET")
        .uri("/api/channels/not-a-valid-uuid")
        .header("X-Dev-User", &created_user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(invalid_uuid_ch_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. バリデーション違反 (空文字メッセージ) -> 400 Bad Request
    let channel_id = uuid::Uuid::new_v4().to_string();
    let empty_msg_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &created_user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "   \n\t  ",
        }))?))?;
    let resp = app.clone().oneshot(empty_msg_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 7. バリデーション違反 (予約語チャンネル名 'general') -> 400 Bad Request
    let reserved_ch_req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &created_user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": "general",
            "description": "General channel",
        }))?))?;
    let resp = app.clone().oneshot(reserved_ch_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 通常チャンネルが DM の内部名を占有できないことを、作成と改名の両方で確認する。
    let target_user_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "id": target_user_id,
            "user_name": format!("target_{}", uuid::Uuid::new_v4().simple()),
            "display_name": "Target",
        }))?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);
    let normal_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &created_user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": normal_id,
            "channel_name": format!("開発_Team-{}", uuid::Uuid::new_v4().simple()),
        }))?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);
    let renamed = format!("相談_Team-{}", uuid::Uuid::new_v4().simple());
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/channels/{normal_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &created_user_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "channel_name": renamed }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    assert_eq!(updated["data"]["channel_name"], renamed);
    let mut ids = [created_user_id.as_str(), target_user_id.as_str()];
    ids.sort();
    let dm_name = format!("dm:{}:{}", ids[0], ids[1]);
    for name in [
        dm_name.as_str(),
        "dm:group:test",
        "dm:",
        "開発:相談",
        "開発😀",
        "開発 相談",
    ] {
        for (method, path) in [
            ("POST", "/api/channels".to_string()),
            ("PATCH", format!("/api/channels/{normal_id}")),
        ] {
            let req = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", &created_user_id)
                .body(Body::from(serde_json::to_vec(
                    &json!({ "channel_name": name }),
                )?))?;
            assert_eq!(
                app.clone().oneshot(req).await?.status(),
                StatusCode::BAD_REQUEST,
                "{method} {name}"
            );
        }
    }
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &created_user_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "target_user_ids": [target_user_id] }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let dm: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    assert_eq!(dm["data"]["channel_name"], dm_name);

    Ok(())
}

#[tokio::test]
async fn test_messages_simple_list() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    unsafe {
        std::env::set_var("DEV_AUTH_ENABLED", "true");
    }

    let config = Config::from_env();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state);

    // 0. ユーザー作成
    let user_id = uuid::Uuid::new_v4().to_string();
    let user_name = format!("msg_user_{}", uuid::Uuid::new_v4().simple());
    let u_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "id": user_id,
            "user_name": user_name,
            "display_name": "Message User",
        }))?))?;
    let resp = app.clone().oneshot(u_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. チャンネル作成
    let channel_name = format!("msg-ch-{}", uuid::Uuid::new_v4().simple());
    let ch_req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": channel_name,
            "description": "Messages test channel",
            "is_private": false,
        }))?))?;
    let resp = app.clone().oneshot(ch_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let ch_body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let channel_id = ch_body["data"]["id"].as_str().unwrap();

    // 2. メッセージを 2 件投稿
    for i in 1..=2 {
        let post_req = Request::builder()
            .method("POST")
            .uri("/api/messages")
            .header("content-type", "application/json")
            .header("X-Dev-User", &user_id)
            .body(Body::from(serde_json::to_vec(&json!({
                "channel_id": channel_id,
                "content": format!("Message {i}"),
            }))?))?;
        let resp = app.clone().oneshot(post_req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 3. GET /api/messages で全件が作成昇順 (ASC) で取得できること
    let list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id}"))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(list_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let messages: Vec<Value> =
        serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "Message 1");
    assert_eq!(messages[1]["content"], "Message 2");

    Ok(())
}
