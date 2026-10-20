use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    auth::DEFAULT_DEV_USER_ID, config::Config, db::init_db, models::UserRole,
    routes::create_router, state::AppState,
};

#[tokio::test]
async fn test_channel_archive_and_unarchive_flow() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    // 1. Alice と Bob 作成
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

    // 2. Alice がチャンネル作成
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("ch_{}", uuid::Uuid::new_v4().simple()),
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

    // 3. 一般ユーザー Bob がアーカイブ試行 -> 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 4. 作成者 Alice がアーカイブ実行 -> 200 OK + txid
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["archive_at"].is_string());
    assert!(json["txid"].is_string());

    // 5. Alice が再度アーカイブ実行 -> no-op で 200 OK かつ txid: null
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["txid"].is_null());

    // 6. アーカイブされたチャンネルへのメッセージ投稿 -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "Cannot post here",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 7. Alice が unarchive 実行 -> 200 OK + txid
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/unarchive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["archive_at"].is_null());
    assert!(json["txid"].is_string());

    // 8. unarchive されたらメッセージ投稿可能
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "Can post now!",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_admin_privileges_and_message_force_delete() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let admin_id = uuid::Uuid::new_v4().to_string();

    // 1. ユーザー作成
    for (id, name) in [
        (&alice_id, "alice"),
        (&bob_id, "bob"),
        (&admin_id, "admin_user"),
    ] {
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

    // 2. admin_id に role_admin を付与
    let mut db_tx = db.clone();
    let mut tx = db_tx.transaction().await?;
    toasty::create!(UserRole {
        tenant_id: backend::auth::tenant::DEFAULT_TENANT_ID,
        user_id: admin_id.as_str(),
        role_id: "role_admin",
    })
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // 3. Alice がチャンネル作成 & メッセージ投稿
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("ch_{}", uuid::Uuid::new_v4().simple()),
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

    let message_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": message_id,
            "channel_id": channel_id,
            "content": "Alice's message",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. 一般ユーザー Bob が Alice のメッセージを削除試行 -> 403 Forbidden
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 5. ワークスペース管理者 Admin が Alice のメッセージを強制削除 -> 200 OK + txid
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["deleted_at"].is_string());
    assert!(json["txid"].is_string());

    // 6. メッセージ2 を作成
    let message_id_2 = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": message_id_2,
            "channel_id": channel_id,
            "content": "Alice's second message",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. Admin が Alice 作成チャンネルをアーカイブ -> 200 OK
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 8. アーカイブ済みチャンネルで、一般ユーザー Bob が Alice のメッセージを削除試行 -> 403 Forbidden (認可先行)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id_2}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 9. アーカイブ済みチャンネルで、Admin は不適切メッセージを強制削除可能 -> 200 OK
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id_2}"))
        .header("X-Dev-User", &admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_member_kick_and_access_revocation() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();

    for (id, name) in [
        (&alice_id, "alice"),
        (&bob_id, "bob"),
        (&charlie_id, "charlie"),
    ] {
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

    // 1. Alice がプライベートチャンネル作成
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": channel_id,
        "channel_name": format!("secret_{}", uuid::Uuid::new_v4().simple()),
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

    // 1.5 作成者 Alice は退出できない -> 400 Bad Request
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/leave"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2. Alice が Bob を追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "user_id": bob_id }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 一般非メンバー Charlie がプライベートチャンネルのメンバーをキック試行 -> 404 Not Found (非公開チャンネル存在隠蔽)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{bob_id}"))
        .header("X-Dev-User", &charlie_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 4. Alice (自身) をキック試行 -> 400 Bad Request
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{alice_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. 一般メンバー Bob がキック試行 -> 403 Forbidden (キック権限なし)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{alice_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 6. 管理者 Admin ユーザー作成
    let admin_id = uuid::Uuid::new_v4().to_string();
    let user_payload = json!({
        "id": admin_id,
        "user_name": format!("admin_{}", uuid::Uuid::new_v4().simple()),
        "display_name": "Admin User",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&user_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let mut db_tx = db.clone();
    let mut tx = db_tx.transaction().await?;
    toasty::create!(UserRole {
        tenant_id: backend::auth::tenant::DEFAULT_TENANT_ID,
        user_id: admin_id.as_str(),
        role_id: "role_admin",
    })
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // Alice が Admin をチャンネルに追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "user_id": admin_id }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. 管理者 Admin であってもチャンネル作成者 Alice をキック不可 -> 403 Forbidden (作成者キック保護)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{alice_id}"))
        .header("X-Dev-User", &admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 8. 作成者 Alice がワークスペース管理者 Admin をキック試行 -> 403 Forbidden (管理者キック保護)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{admin_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 8. 作成者 Alice が Bob をキック実行 -> 200 OK + txid
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{bob_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["user_id"], bob_id);
    assert!(json["txid"].is_string());

    // 9. キックされた Bob がプライベートチャンネルのメッセージ Shape にアクセス -> 404 (アクセス権喪失)
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/shapes/messages?channel_id={channel_id}&offset=-1"
        ))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn test_get_me_reports_is_admin() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let member_id = uuid::Uuid::new_v4().to_string();
    let admin_id = uuid::Uuid::new_v4().to_string();

    for (id, name) in [(&member_id, "member"), (&admin_id, "admin")] {
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

    // 一般メンバー: is_admin = false
    let req = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("X-Dev-User", &member_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["id"], member_id);
    assert_eq!(json["is_admin"], false);
    // 既存フィールドがフラットに残っていること
    assert_eq!(json["display_name"], "member");

    // role_admin を付与
    let mut db_tx = db.clone();
    let mut tx = db_tx.transaction().await?;
    toasty::create!(UserRole {
        tenant_id: backend::auth::tenant::DEFAULT_TENANT_ID,
        user_id: admin_id.as_str(),
        role_id: "role_admin",
    })
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // ワークスペース管理者: is_admin = true
    let req = Request::builder()
        .method("GET")
        .uri("/api/users/me")
        .header("X-Dev-User", &admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["id"], admin_id);
    assert_eq!(json["is_admin"], true);

    // キック可否の UI 判定用の管理者一覧にも同じ条件で載る
    let req = Request::builder()
        .method("GET")
        .uri("/api/users/admins")
        .header("X-Dev-User", &member_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let ids = json["user_ids"]
        .as_array()
        .expect("user_ids should be an array");
    assert!(ids.iter().any(|id| id.as_str() == Some(admin_id.as_str())));
    assert!(
        ids.iter()
            .any(|id| id.as_str() == Some(DEFAULT_DEV_USER_ID))
    );
    assert!(!ids.iter().any(|id| id.as_str() == Some(member_id.as_str())));

    Ok(())
}
