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

    // 1. バリデーション違反 (予約語のユーザー名) -> 400 Bad Request
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

    // 2. 正常なユーザー作成 -> 200 OK
    let valid_user_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "user_name": user_name,
            "display_name": "Normal User",
        }))?))?;
    let resp = app.clone().oneshot(valid_user_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

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

    // 4. バリデーション違反 (空文字メッセージ) -> 400 Bad Request
    let channel_id = uuid::Uuid::new_v4().to_string();
    let empty_msg_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "   ",
        }))?))?;
    let resp = app.clone().oneshot(empty_msg_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. バリデーション違反 (予約語チャンネル名 'general') -> 400 Bad Request
    let reserved_ch_req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": "general",
            "description": "General channel",
        }))?))?;
    let resp = app.clone().oneshot(reserved_ch_req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_messages_cursor_pagination() -> anyhow::Result<()> {
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
    let user_name = format!("page_user_{}", uuid::Uuid::new_v4().simple());
    let u_req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "id": user_id,
            "user_name": user_name,
            "display_name": "Pagination User",
        }))?))?;
    let resp = app.clone().oneshot(u_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. チャンネル作成
    let channel_name = format!("page_ch_{}", uuid::Uuid::new_v4().simple());
    let ch_req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": channel_name,
            "description": "Pagination test channel",
            "is_private": false,
        }))?))?;
    let resp = app.clone().oneshot(ch_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let ch_body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    let channel_id = ch_body["data"]["id"].as_str().unwrap();

    // 2. メッセージを 5 件連続投稿 (時刻差をつけるため小さなスリープ)
    let mut msg_ids = Vec::new();
    let mut msg_created_ats = Vec::new();

    for i in 1..=5 {
        tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;
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
        let val: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
        msg_ids.push(val["data"]["id"].as_str().unwrap().to_string());
        msg_created_ats.push(val["data"]["created_at"].as_str().unwrap().to_string());
    }

    // 3. ページネーション: limit = 2 で取得 (最新 2 件: Message 5, Message 4)
    let list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id}&limit=2"))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(list_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let page1: Vec<Value> = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await?)?;
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0]["content"], "Message 5");
    assert_eq!(page1[1]["content"], "Message 4");

    // 4. ページネーション: before を Message 4 の created_at にして次ページ 2 件取得 (Message 3, Message 2)
    let before_cursor = &page1[1]["created_at"].as_str().unwrap();
    let list_req2 = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/messages?channel_id={channel_id}&before={before_cursor}&limit=2"
        ))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp2 = app.clone().oneshot(list_req2).await?;
    assert_eq!(resp2.status(), StatusCode::OK);
    let page2: Vec<Value> =
        serde_json::from_slice(&to_bytes(resp2.into_body(), usize::MAX).await?)?;
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0]["content"], "Message 3");
    assert_eq!(page2[1]["content"], "Message 2");

    Ok(())
}
