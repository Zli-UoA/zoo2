use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

#[tokio::test]
async fn test_create_user_and_message_returns_txid() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    unsafe {
        std::env::set_var("DEV_AUTH_ENABLED", "true");
    }

    let config = Config::from_env();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let user_id = uuid::Uuid::new_v4().to_string();
    let user_name = format!("test_user_{}", uuid::Uuid::new_v4().simple());

    // 1. ユーザー作成
    let user_payload = json!({
        "id": user_id,
        "user_name": user_name,
        "display_name": "Test User",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json.get("data").is_some());
    let txid = json["txid"].as_str().expect("txid should be string");
    assert!(!txid.is_empty());
    println!("Created user with txid: {txid}");

    // 2. チャンネル作成
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_name = format!("test-ch-{}", uuid::Uuid::new_v4().simple());
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": channel_name,
        "description": "Test Channel",
        "is_private": false,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let ch_txid = json["txid"].as_str().expect("txid should be string");
    assert!(!ch_txid.is_empty());
    println!("Created channel with txid: {ch_txid}");

    // 2b. チャンネル作成の冪等性: 同一 channel_id かつ同一内容で再度 POST した場合、200 OK で作成済みチャンネルが返ること
    let req_resend = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;
    let resp_resend = app.clone().oneshot(req_resend).await?;
    assert_eq!(resp_resend.status(), StatusCode::OK);
    let body_resend = to_bytes(resp_resend.into_body(), usize::MAX).await?;
    let json_resend: Value = serde_json::from_slice(&body_resend)?;
    assert_eq!(json_resend["data"]["id"], channel_id);

    // 2c. チャンネル作成で同じ ID だが異なる内容の場合、409 Conflict になること
    let conflict_payload = json!({
        "id": channel_id,
        "channel_name": format!("diff-{}", uuid::Uuid::new_v4().simple()),
        "description": "Test Channel",
        "is_private": false,
    });
    let req_conflict = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&conflict_payload)?))?;
    let resp_conflict = app.clone().oneshot(req_conflict).await?;
    assert_eq!(resp_conflict.status(), StatusCode::CONFLICT);

    // 2d. ゲスト検証順序: ゲストがチャンネル作成を試みた場合、既存 ID でも新規 ID でも同じ 403 Forbidden になり、チャンネルの存在が漏洩しないこと
    let guest_id = uuid::Uuid::new_v4().to_string();
    let guest_name = format!("guest_{}", uuid::Uuid::new_v4().simple());
    let guest_payload = json!({
        "id": guest_id,
        "user_name": guest_name,
        "display_name": "Guest User",
    });
    let req_guest_user = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&guest_payload)?))?;
    let resp_guest_user = app.clone().oneshot(req_guest_user).await?;
    assert_eq!(resp_guest_user.status(), StatusCode::OK);

    let mut conn = state.db.clone();
    toasty::sql::statement("UPDATE users SET guest_kind = 'multi' WHERE id = $1")
        .bind(guest_id.as_str())
        .exec(&mut conn)
        .await?;

    // 既存 ID を指定したゲストのリクエスト -> 403 (409 ではなく 403)
    let req_guest_existing = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &guest_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;
    let resp_guest_existing = app.clone().oneshot(req_guest_existing).await?;
    assert_eq!(resp_guest_existing.status(), StatusCode::FORBIDDEN);

    // 未使用の新規 ID を指定したゲストのリクエスト -> 403
    let unused_channel_id = uuid::Uuid::new_v4().to_string();
    let unused_payload = json!({
        "id": unused_channel_id,
        "channel_name": format!("guest-{}", uuid::Uuid::new_v4().simple()),
        "description": "Guest Channel",
        "is_private": false,
    });
    let req_guest_unused = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &guest_id)
        .body(Body::from(serde_json::to_vec(&unused_payload)?))?;
    let resp_guest_unused = app.clone().oneshot(req_guest_unused).await?;
    assert_eq!(resp_guest_unused.status(), StatusCode::FORBIDDEN);

    // 3. メッセージ送信
    let message_id = uuid::Uuid::new_v4().to_string();
    let message_payload = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Hello World with txid!",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let msg_txid = json["txid"].as_str().expect("txid should be string");
    assert!(!msg_txid.is_empty());
    println!("Created message with txid: {msg_txid}");

    // 4. メッセージ編集
    let update_payload = json!({
        "content": "Updated content!",
    });

    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/messages/{message_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&update_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["content"], "Updated content!");
    let update_txid = json["txid"].as_str().expect("txid should be string");
    assert!(!update_txid.is_empty());

    // 5. メッセージ論理削除
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["deleted_at"].is_string());
    let delete_txid = json["txid"].as_str().expect("txid should be string");
    assert!(!delete_txid.is_empty());

    Ok(())
}
