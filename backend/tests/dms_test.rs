use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    auth::DEFAULT_DEV_USER_ID, config::Config, db::init_db, routes::create_router, state::AppState,
};

async fn setup_app() -> anyhow::Result<(axum::Router, AppState)> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state.clone());
    Ok((app, state))
}

async fn create_test_user(app: &axum::Router, id: &str, name: &str) -> anyhow::Result<()> {
    let payload = json!({
        "id": id,
        "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
        "display_name": name,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn test_direct_message_1on1_flow() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. Alice が Bob との 1対1 DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();
    let channel_name = json["data"]["channel_name"].as_str().unwrap().to_string();

    assert_eq!(json["data"]["is_dm"], true);
    assert_eq!(json["data"]["is_private"], true);
    assert!(json["txid"].is_string());

    // チャンネル名が dm:<min_uuid>:<max_uuid> 形式であること
    let (min_u, max_u) = if alice_id < bob_id {
        (&alice_id, &bob_id)
    } else {
        (&bob_id, &alice_id)
    };
    assert_eq!(channel_name, format!("dm:{min_u}:{max_u}"));

    // 2. Bob が Alice との DM を取得 (既存取得・冪等性)
    let dm_payload_bob = json!({
        "target_user_ids": [&alice_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&dm_payload_bob)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    // 既に所属中のため WAL 書込なし -> txid: null
    assert!(json["txid"].is_null());

    // 3. Alice が再度呼んでも同一チャンネルかつ txid: null
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    assert!(json["txid"].is_null());

    Ok(())
}

#[tokio::test]
async fn test_direct_message_leave_and_reopen() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. Alice が Bob との 1対1 DM を作成 (Alice が created_user_id)
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();

    // 2. 作成者 Alice が DM から退出 -> DM では作成者であっても退出可能 (200 OK)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/leave"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 退出した Alice はチャンネル情報取得で 404 (存在隠蔽)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Bob は所属したままなので 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Alice が再度 Bob との DM をリクエスト (再オープン・復帰)
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    // channel_user への INSERT が発生したため有効な txid が返却される
    assert!(json["txid"].is_string());

    // 復帰した Alice は再度チャンネル情報取得可能
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_group_dm_flow() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;
    create_test_user(&app, &charlie_id, "charlie").await?;

    // 1. Alice が Bob, Charlie とのグループ DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id, &charlie_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();
    let channel_name = json["data"]["channel_name"].as_str().unwrap().to_string();

    assert_eq!(json["data"]["is_dm"], true);
    assert_eq!(json["data"]["is_private"], true);
    assert!(channel_name.starts_with("dm:group:"));
    assert!(json["txid"].is_string());

    // 2. 参加者全員がチャンネル取得可能
    for uid in [&alice_id, &bob_id, &charlie_id] {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/channels/{channel_id}"))
            .header("X-Dev-User", uid)
            .body(Body::empty())?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    Ok(())
}

#[tokio::test]
async fn test_dm_validation_errors() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. target_user_ids が空
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": []
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2. target_user_ids に自身が含まれている
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [&alice_id]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 3. target_user_ids に重複がある
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [&bob_id, &bob_id]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 4. target_user_ids が 9 人を超える (自身含め最大 10 人)
    let too_many_users: Vec<String> = (0..10).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": too_many_users
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. 存在しないユーザーを指定
    let non_existent_user = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [non_existent_user]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. 不正な UUID
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": ["invalid-uuid"]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn test_dm_access_control_and_privacy() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();
    let admin_id = DEFAULT_DEV_USER_ID; // 開発環境の特権管理者

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;
    create_test_user(&app, &charlie_id, "charlie").await?;

    // Alice と Bob の DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();

    // Alice がメッセージを投稿
    let msg_payload = json!({
        "channel_id": &channel_id,
        "content": "Secret message between Alice and Bob",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let msg_json: Value = serde_json::from_slice(&body)?;
    let message_id = msg_json["data"]["id"].as_str().unwrap().to_string();

    // 1. ワークスペース管理者 Admin (非参加) による存在隠蔽検証 (すべて 404 Not Found)
    // - チャンネル閲覧
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネル更新試行
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/channels/{channel_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": "renamed_by_admin"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネル削除試行
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネルアーカイブ試行
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メンバー追加試行
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "user_id": admin_id
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ閲覧試行
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ投稿試行
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": &channel_id,
            "content": "Admin snooping"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ削除試行
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 2. 参加者 (Alice) であっても DM に対して禁止されている操作 (すべて 403 Forbidden)
    // - 更新
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/channels/{channel_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "description": "forbidden update"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - 削除
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - アーカイブ
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - メンバー追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "user_id": &charlie_id
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - メンバーキック
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{bob_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - 直接 join 試行 (Charlie による)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/join"))
        .header("X-Dev-User", &charlie_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 3. メッセージ削除制御
    // - Bob が Alice のメッセージを削除試行 -> 403 Forbidden (DM では投稿者本人のみ削除可能)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - Alice 本人が削除 -> 200 OK
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}
