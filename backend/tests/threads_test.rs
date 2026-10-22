use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

#[tokio::test]
async fn test_thread_reply_creation_and_validation() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();

    // 1. Alice 作成
    let user_payload = json!({
        "id": alice_id,
        "user_name": format!("alice_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Alice",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. チャンネル 1 作成
    let channel_id_1 = uuid::Uuid::new_v4().to_string();
    let channel_payload_1 = json!({
        "id": channel_id_1,
        "channel_name": format!("ch1-{}", uuid::Uuid::new_v4().simple()),
        "is_private": false,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload_1)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 親メッセージ (Root) 投稿
    let root_message_id = uuid::Uuid::new_v4().to_string();
    let root_payload = json!({
        "id": root_message_id,
        "channel_id": channel_id_1,
        "content": "Root message",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&root_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["parent_id"].is_null());
    assert!(json["txid"].is_string());

    // 4. 親メッセージへのスレッド返信 1 を投稿
    let reply_1_id = uuid::Uuid::new_v4().to_string();
    let reply_1_payload = json!({
        "id": reply_1_id,
        "channel_id": channel_id_1,
        "content": "Reply 1 to root",
        "parent_id": root_message_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&reply_1_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["parent_id"], root_message_id);
    assert!(json["txid"].is_string());

    // 5. スレッド返信 1 に対する再返信 (多段ネスト) を試行 -> 400 Bad Request
    let nested_reply_payload = json!({
        "channel_id": channel_id_1,
        "content": "Nested reply to reply 1",
        "parent_id": reply_1_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&nested_reply_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. チャンネル 2 作成
    let channel_id_2 = uuid::Uuid::new_v4().to_string();
    let channel_payload_2 = json!({
        "id": channel_id_2,
        "channel_name": format!("ch2-{}", uuid::Uuid::new_v4().simple()),
        "is_private": false,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload_2)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. チャンネル 2 でチャンネル 1 の root_message_id を指定して投稿試行 -> 404 Not Found (非公開メッセージの存在漏洩防止)
    let cross_channel_payload = json!({
        "channel_id": channel_id_2,
        "content": "Cross channel reply",
        "parent_id": root_message_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&cross_channel_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 8. parent_id が不正な UUID の場合 -> 400 Bad Request
    let invalid_uuid_payload = json!({
        "channel_id": channel_id_1,
        "content": "Invalid parent UUID",
        "parent_id": "not-a-valid-uuid",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&invalid_uuid_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 9. 自メッセージ自身を親として指定した場合 (id == parent_id) -> 404 Not Found
    let self_id = uuid::Uuid::new_v4().to_string();
    let self_parent_payload = json!({
        "id": self_id,
        "channel_id": channel_id_1,
        "content": "Self parent message",
        "parent_id": self_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&self_parent_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 10. 親メッセージを論理削除
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{root_message_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 11. 削除済み親メッセージへの返信投稿試行 -> 404 Not Found
    let reply_to_deleted_payload = json!({
        "channel_id": channel_id_1,
        "content": "Reply to deleted parent",
        "parent_id": root_message_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&reply_to_deleted_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 10. 親メッセージ削除後も既存の子返信は保持されていること (GET /api/messages)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id_1}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let messages: Vec<Value> = serde_json::from_slice(&body)?;
    let reply_found = messages.iter().find(|m| m["id"] == reply_1_id);
    assert!(reply_found.is_some(), "Reply 1 should still exist");
    assert_eq!(reply_found.unwrap()["parent_id"], root_message_id);

    Ok(())
}
