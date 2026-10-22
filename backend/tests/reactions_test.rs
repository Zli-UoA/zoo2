use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    config::Config,
    db::init_db,
    models::{Channel, now_utc_millis},
    routes::create_router,
    state::AppState,
};

#[tokio::test]
async fn test_reaction_flow_and_txid() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    // 1. Alice と Bob を作成
    for (id, name) in [(&alice_id, "alice"), (&bob_id, "bob")] {
        let user_payload = json!({
            "id": id,
            "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
            "display_name": name,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 2. 公開チャンネル作成
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("pub-{}", uuid::Uuid::new_v4().simple()),
        "description": "Public Channel",
        "is_private": false,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. Alice がメッセージ投稿
    let message_id = uuid::Uuid::new_v4().to_string();
    let message_payload = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Message for reactions testing",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Alice が 👍 リアクションを追加
    let reaction_payload = json!({ "emoji": "👍" });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&reaction_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["emoji"], "👍");
    assert_eq!(json["data"]["user_id"], alice_id);
    assert_eq!(json["data"]["message_id"], message_id);
    let alice_reaction_id = json["data"]["id"].as_str().unwrap().to_string();
    let txid = json["txid"]
        .as_str()
        .expect("txid must be string on insert");
    assert!(!txid.is_empty());

    // 5. Alice が同一の 👍 リアクションを再度追加 (重複・冪等性確認)
    // -> no-op のため 200 OK かつ txid: null であること
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&reaction_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], alice_reaction_id);
    assert!(json["txid"].is_null(), "txid must be null on duplicate add");

    // 6. Bob が同一メッセージに 👍 リアクションをクライアント指定 ID で追加
    let client_reaction_id = uuid::Uuid::new_v4().to_string();
    let bob_reaction_payload = json!({
        "id": client_reaction_id,
        "emoji": "👍"
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&bob_reaction_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["user_id"], bob_id);
    assert_eq!(json["data"]["id"], client_reaction_id);
    assert!(json["txid"].is_string());

    // 7. Alice が 👍 リアクションを削除 (クエリパラメータ形式)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/messages/{message_id}/reactions?emoji=%F0%9F%91%8D"
        ))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], alice_reaction_id);
    assert!(json["txid"].is_string());

    // 8. Alice が既に削除した 👍 を再度削除試行 -> 404 Not Found
    let req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/messages/{message_id}/reactions?emoji=%F0%9F%91%8D"
        ))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 9. Bob のリアクション 👍 がまだ残っている状態で、Alice が再度 DELETE を試行 -> 404 Not Found (他人のリアクションは削除できない)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/messages/{message_id}/reactions?emoji=%F0%9F%91%8D"
        ))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 10. 不正な UUID を id として渡した場合は重複時であっても 400 Bad Request になること
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": "not-a-valid-uuid",
            "emoji": "👍",
        }))?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 11. Bob 本人が自身の 👍 リアクションを削除 -> 200 OK
    let req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/messages/{message_id}/reactions?emoji=%F0%9F%91%8D"
        ))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_reaction_normalization_and_formats() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let user_id = uuid::Uuid::new_v4().to_string();
    let user_payload = json!({
        "id": user_id,
        "user_name": format!("user_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Test User",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("ch-{}", uuid::Uuid::new_v4().simple()),
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

    let message_id = uuid::Uuid::new_v4().to_string();
    let message_payload = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Test message",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. VS16 付きハート (❤️) を追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "❤️" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. VS16 なしハート (❤) で再度追加 -> 正規化により重複と判定され txid: null が返ること
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "❤" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["txid"].is_null());

    // 3. ショートコード形式 (:party_parrot:)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "emoji": ":party_parrot:" }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Emoji Kitchen 形式 (kitchen:1f600_1f431)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "emoji": "kitchen:1f600_1f431" }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. 不正な絵文字文字列 (通常のアルファベット) -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "emoji": "invalid_text" }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_reaction_permissions_and_archived_channel() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();

    for (id, name) in [(&alice_id, "alice"), (&charlie_id, "charlie")] {
        let user_payload = json!({
            "id": id,
            "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
            "display_name": name,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Alice がプライベートチャンネル作成 (Alice のみ所属)
    let private_channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": private_channel_id,
        "channel_name": format!("priv-{}", uuid::Uuid::new_v4().simple()),
        "is_private": true,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let message_id = uuid::Uuid::new_v4().to_string();
    let message_payload = json!({
        "id": message_id,
        "channel_id": private_channel_id,
        "content": "Secret message",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. Charlie (非メンバー) がプライベートチャンネルのメッセージにリアクション追加 -> 404 (秘匿)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &charlie_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "👍" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 2. チャンネルをアーカイブ状態にする
    let mut db_tx = db.clone();
    let mut tx = db_tx.transaction().await?;
    Channel::update_by_id(&private_channel_id)
        .archive_at(Some(now_utc_millis()))
        .exec(&mut tx)
        .await?;
    tx.commit().await?;

    // 3. アーカイブされたチャンネルへのリアクション追加 -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "👍" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 4. アーカイブされたチャンネルへのメッセージ投稿も禁止 -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": private_channel_id,
            "content": "Should fail",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_concurrent_reaction_idempotency() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let user_id = uuid::Uuid::new_v4().to_string();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let message_id = uuid::Uuid::new_v4().to_string();

    let user_payload = json!({
        "id": user_id,
        "user_name": format!("user_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Concurrent User",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("ch-{}", uuid::Uuid::new_v4().simple()),
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

    let message_payload = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Concurrent reaction target",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // tokio::join! で同時に 2 本同一のリアクションを追加リクエスト
    let reaction_payload = json!({ "emoji": "🎉" });
    let req1 = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&reaction_payload)?))?;

    let req2 = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&reaction_payload)?))?;

    let app1 = app.clone();
    let app2 = app.clone();

    let (resp1, resp2) = tokio::join!(app1.oneshot(req1), app2.oneshot(req2));
    let resp1 = resp1?;
    let resp2 = resp2?;

    // 両方とも 200 OK で返ること
    assert_eq!(resp1.status(), StatusCode::OK);
    assert_eq!(resp2.status(), StatusCode::OK);

    let body1 = to_bytes(resp1.into_body(), usize::MAX).await?;
    let body2 = to_bytes(resp2.into_body(), usize::MAX).await?;

    let json1: Value = serde_json::from_slice(&body1)?;
    let json2: Value = serde_json::from_slice(&body2)?;

    // 同一のリアクションレコードが返ること
    assert_eq!(json1["data"]["id"], json2["data"]["id"]);
    assert_eq!(json1["data"]["emoji"], "🎉");

    // どちらか一方は挿入成功で txid (文字列) を持ち、他方は重複扱いで txid: null であること
    let has_txid1 = json1["txid"].is_string();
    let has_txid2 = json2["txid"].is_string();
    let is_null1 = json1["txid"].is_null();
    let is_null2 = json2["txid"].is_null();

    assert!(
        (has_txid1 && is_null2) || (is_null1 && has_txid2),
        "Exactly one response must have txid string and the other must have null txid"
    );

    Ok(())
}

#[tokio::test]
async fn test_reaction_on_deleted_message() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let user_id = uuid::Uuid::new_v4().to_string();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let message_id = uuid::Uuid::new_v4().to_string();

    let user_payload = json!({
        "id": user_id,
        "user_name": format!("user_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Test User",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("ch-{}", uuid::Uuid::new_v4().simple()),
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

    let message_payload = json!({
        "id": message_id,
        "channel_id": channel_id,
        "content": "Message to be deleted",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&message_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. リアクションを1件追加しておく
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "👍" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. メッセージを削除 (論理削除)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 削除済みメッセージにリアクション追加試行 -> 404 Not Found
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "❤️" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 4. 削除済みメッセージのリアクション削除試行 -> 404 Not Found
    let req = Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/messages/{message_id}/reactions?emoji=%F0%9F%91%8D"
        ))
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn test_reaction_kitchen_validation_and_pk_conflict() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let user_id = uuid::Uuid::new_v4().to_string();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let message_id = uuid::Uuid::new_v4().to_string();

    // ユーザー作成
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "id": user_id,
            "user_name": format!("user_{}", uuid::Uuid::new_v4().simple()),
            "display_name": "User",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 公開チャンネル作成
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": channel_id,
            "channel_name": format!("pub-{}", uuid::Uuid::new_v4().simple()),
            "is_private": false,
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // メッセージ作成
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": message_id,
            "channel_id": channel_id,
            "content": "Emoji kitchen & PK conflict test message",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. 不正な Emoji Kitchen 形式はすべて 400 Bad Request
    let invalid_kitchens = [
        "kitchen:hello",
        "kitchen:a_b_c",
        "kitchen:0_1",
        "kitchen:hello_world",
        "kitchen:_1f600",
        "kitchen:1f600_",
    ];
    for invalid in invalid_kitchens {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/messages/{message_id}/reactions"))
            .header("content-type", "application/json")
            .header("X-Dev-User", &user_id)
            .body(Body::from(serde_json::to_vec(&json!({
                "emoji": invalid
            }))?))?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "Expected 400 Bad Request for {invalid}"
        );
    }

    // 2. 正当な Emoji Kitchen は 200 OK で正規化されて保存
    let reaction_id_1 = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": reaction_id_1,
            "emoji": "kitchen:1f600_1f431"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let res_json: Value = serde_json::from_slice(&body)?;
    assert_eq!(res_json["data"]["emoji"], "kitchen:1f431_1f600");

    // 3. 既存の reaction_id_1 と同じ ID を指定して別の絵文字で POST を試みる -> 409 Conflict
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{message_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &user_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": reaction_id_1,
            "emoji": "👍"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "Expected 409 Conflict on PK collision"
    );

    Ok(())
}
