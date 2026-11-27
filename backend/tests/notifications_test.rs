use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    config::Config, db::init_db, models::Notification, routes::create_router, state::AppState,
};

#[tokio::test]
async fn test_notifications_full_flow() -> anyhow::Result<()> {
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

    let bob_username = format!("bob_{}", uuid::Uuid::new_v4().simple());
    let charlie_username = format!("charlie_{}", uuid::Uuid::new_v4().simple());

    // 1. ユーザー作成 (Alice, Bob, Charlie)
    for (id, username, display) in [
        (
            &alice_id,
            format!("alice_{}", uuid::Uuid::new_v4().simple()),
            "Alice",
        ),
        (&bob_id, bob_username.clone(), "Bob"),
        (&charlie_id, charlie_username.clone(), "Charlie"),
    ] {
        let user_payload = json!({
            "id": id,
            "user_name": username,
            "display_name": display,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 2. 公開チャンネル作成 (Alice が作成)
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
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Bob と Charlie をチャンネルに追加
    for uid in [&bob_id, &charlie_id] {
        let join_payload = json!({ "user_id": uid });
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel_id}/users"))
            .header("content-type", "application/json")
            .header("X-Dev-User", &alice_id)
            .body(Body::from(serde_json::to_vec(&join_payload)?))?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 3. メンション通知テスト: Alice が @bob をメンション
    let msg1_id = uuid::Uuid::new_v4().to_string();
    let msg1_payload = json!({
        "id": msg1_id,
        "channel_id": channel_id,
        "content": format!("Hello @{} how are you?", bob_username),
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg1_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // DB 検証: Bob 宛ての通知が作成されていること
    let mut check_db = db.clone();
    let bob_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(bob_id.as_str())
            .and(Notification::fields().message_id().eq(msg1_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(bob_notifs.len(), 1);
    assert_eq!(bob_notifs[0].notification_type, "mention");
    assert_eq!(bob_notifs[0].actor_id, alice_id);
    assert!(!bob_notifs[0].is_read);

    // Charlie や Alice には通知が作成されていないこと
    let charlie_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(charlie_id.as_str())
            .and(Notification::fields().message_id().eq(msg1_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(charlie_notifs.len(), 0);

    // 4. 重複排除テスト: Alice が @channel と @bob を両方メンション
    let msg2_id = uuid::Uuid::new_v4().to_string();
    let msg2_payload = json!({
        "id": msg2_id,
        "channel_id": channel_id,
        "content": format!("Attention @channel especially @{} !", bob_username),
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg2_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Bob と Charlie にそれぞれ 1 件ずつ通知が作成され、重複しないこと
    let bob_msg2_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(bob_id.as_str())
            .and(Notification::fields().message_id().eq(msg2_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(bob_msg2_notifs.len(), 1);
    assert_eq!(bob_msg2_notifs[0].notification_type, "mention");

    let charlie_msg2_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(charlie_id.as_str())
            .and(Notification::fields().message_id().eq(msg2_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(charlie_msg2_notifs.len(), 1);
    assert_eq!(charlie_msg2_notifs[0].notification_type, "mention");

    // 5. スレッド返信通知テスト: Bob が親メッセージを投稿し、Charlie がそれに返信
    let bob_root_msg_id = uuid::Uuid::new_v4().to_string();
    let bob_root_payload = json!({
        "id": bob_root_msg_id,
        "channel_id": channel_id,
        "content": "Bob's root discussion",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&bob_root_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let reply_id = uuid::Uuid::new_v4().to_string();
    let reply_payload = json!({
        "id": reply_id,
        "channel_id": channel_id,
        "content": "Replying to your message",
        "parent_id": bob_root_msg_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &charlie_id)
        .body(Body::from(serde_json::to_vec(&reply_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let bob_reply_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(bob_id.as_str())
            .and(Notification::fields().message_id().eq(reply_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(bob_reply_notifs.len(), 1);
    assert_eq!(bob_reply_notifs[0].notification_type, "reply");
    assert_eq!(bob_reply_notifs[0].actor_id, charlie_id);

    // 6. リアクション通知テスト: Charlie の返信メッセージに Alice がリアクション追加
    let react_payload = json!({ "emoji": "👍" });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{reply_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&react_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let charlie_reaction_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(charlie_id.as_str())
            .and(Notification::fields().message_id().eq(reply_id.as_str()))
            .and(Notification::fields().notification_type().eq("reaction")),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(charlie_reaction_notifs.len(), 1);
    assert_eq!(charlie_reaction_notifs[0].actor_id, alice_id);

    // 7. [P2] 既読化 API テスト: 単一既読
    let target_notif = &bob_notifs[0];
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/notifications/{}/read", target_notif.id))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["is_read"], true);
    assert!(json["txid"].is_string());

    // 既に既読の状態で再度呼び出す -> [P2] txid: null
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/notifications/{}/read", target_notif.id))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["is_read"], true);
    assert!(json["txid"].is_null()); // no-op

    // 8. [P2] 既読化 API テスト: 一括既読 (read-all)
    // Charlie にはまだ未読がある
    let req = Request::builder()
        .method("POST")
        .uri("/api/notifications/read-all")
        .header("X-Dev-User", &charlie_id)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({}))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert!(json["data"]["updated_count"].as_u64().unwrap() >= 1);
    assert!(json["txid"].is_string());

    // 未読が 0 件の状態で再度 read-all -> [P2] txid: null
    let req = Request::builder()
        .method("POST")
        .uri("/api/notifications/read-all")
        .header("X-Dev-User", &charlie_id)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({}))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["updated_count"], 0);
    assert!(json["txid"].is_null()); // 0件更新なので null

    // 9. [P1] プライベートチャンネルでのキック・退出後の通知防止テスト
    let priv_channel_id = uuid::Uuid::new_v4().to_string();
    let priv_channel_payload = json!({
        "id": priv_channel_id,
        "channel_name": format!("priv-{}", uuid::Uuid::new_v4().simple()),
        "is_private": true,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&priv_channel_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Bob を追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{priv_channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(
            &json!({ "user_id": &bob_id }),
        )?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Bob がメッセージ投稿
    let bob_priv_msg_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": bob_priv_msg_id,
            "channel_id": priv_channel_id,
            "content": "Secret message from Bob",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Alice が Bob をキック
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{priv_channel_id}/users/{bob_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Alice が Bob の過去メッセージに返信
    let reply_to_kicked_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": reply_to_kicked_id,
            "channel_id": priv_channel_id,
            "content": "Reply after kick",
            "parent_id": bob_priv_msg_id,
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Alice が Bob の過去メッセージにリアクション
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/messages/{bob_priv_msg_id}/reactions"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({ "emoji": "🎉" }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // [P1] 検証: Bob はキック済みのため、プライベートチャンネルの返信およびリアクション通知が作成されていないこと
    let kicked_reply_notifs = Notification::filter(
        Notification::fields().user_id().eq(bob_id.as_str()).and(
            Notification::fields()
                .message_id()
                .eq(reply_to_kicked_id.as_str()),
        ),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(kicked_reply_notifs.len(), 0);

    let kicked_react_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(bob_id.as_str())
            .and(
                Notification::fields()
                    .message_id()
                    .eq(bob_priv_msg_id.as_str()),
            )
            .and(Notification::fields().notification_type().eq("reaction")),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(kicked_react_notifs.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_notifications_p2_p3_fixes() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let dave_id = uuid::Uuid::new_v4().to_string(); // 非メンバー
    let bob_id = uuid::Uuid::new_v4().to_string();

    let bob_username = format!("bob_{}", uuid::Uuid::new_v4().simple());
    let dave_username = format!("dave_{}", uuid::Uuid::new_v4().simple());

    // ユーザー作成
    for (id, username, display) in [
        (
            &alice_id,
            format!("alice_{}", uuid::Uuid::new_v4().simple()),
            "Alice",
        ),
        (&dave_id, dave_username.clone(), "Dave"),
        (&bob_id, bob_username.clone(), "Bob"),
    ] {
        let user_payload = json!({
            "id": id,
            "user_name": username,
            "display_name": display,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 公開チャンネル作成 (Alice が作成、Alice のみ所属)
    let public_ch_id = uuid::Uuid::new_v4().to_string();
    let ch_payload = json!({
        "id": public_ch_id,
        "channel_name": format!("pub-{}", uuid::Uuid::new_v4().simple()),
        "is_private": false,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&ch_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. [P2] 非メンバー Dave が公開チャンネルに投稿
    let dave_root_msg_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &dave_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": dave_root_msg_id,
            "channel_id": public_ch_id,
            "content": "Dave's public message without joining channel_user",
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Alice が Dave の投稿に返信
    let reply_msg_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": reply_msg_id,
            "channel_id": public_ch_id,
            "content": "Alice replies to Dave",
            "parent_id": dave_root_msg_id,
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // [P2 検証]: 非メンバー Dave 宛てに reply 通知が作成されていること
    let mut check_db = db.clone();
    let dave_reply_notifs = Notification::filter(
        Notification::fields().user_id().eq(dave_id.as_str()).and(
            Notification::fields()
                .message_id()
                .eq(reply_msg_id.as_str()),
        ),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(dave_reply_notifs.len(), 1);
    assert_eq!(dave_reply_notifs[0].notification_type, "reply");
    assert_eq!(dave_reply_notifs[0].actor_id, alice_id);

    // Bob をチャンネルに参加させる (Bob はメンバー、Dave は非メンバー)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{public_ch_id}/join"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. [P3] 文末句読点付きメンション & 日本語句読点境界 & メール・URL 除外 & 非メンバーメンション除外テスト
    // Alice が「こんにちは。Thanks @bob. @dave Please email test@example.com or visit https://example.com/@here」を投稿
    let p3_msg_id = uuid::Uuid::new_v4().to_string();
    let p3_content = format!(
        "こんにちは。Thanks @{}. @{} Please email test@example.com or visit https://example.com/@here",
        bob_username, dave_username
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": p3_msg_id,
            "channel_id": public_ch_id,
            "content": p3_content,
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // [P3 検証 1]: メンバー Bob 宛てに mention 通知が作成されていること (句点直後・文末ピリオド両方の抽出成功)
    let bob_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(bob_id.as_str())
            .and(Notification::fields().message_id().eq(p3_msg_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(bob_notifs.len(), 1);
    assert_eq!(bob_notifs[0].notification_type, "mention");

    // [P2 検証 2]: 非メンバー Dave 宛てには mention 通知が作成されないこと (チャンネル所属メンバー限定)
    let dave_mention_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(dave_id.as_str())
            .and(Notification::fields().message_id().eq(p3_msg_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(dave_mention_notifs.len(), 0);

    // [P3 検証 3]: @here が URL の一部にあるため、全メンバーへの @here 通知は作成されないこと (Alice/Bob/Dave に @here 通知なし)
    let alice_here_notifs = Notification::filter(
        Notification::fields()
            .user_id()
            .eq(alice_id.as_str())
            .and(Notification::fields().message_id().eq(p3_msg_id.as_str())),
    )
    .exec(&mut check_db)
    .await?;
    assert_eq!(alice_here_notifs.len(), 0);

    // 3. read-all クエリパラメータ (?channel_id=...) テスト
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/notifications/read-all?channel_id={public_ch_id}"
        ))
        .header("X-Dev-User", &dave_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["updated_count"], 1);
    assert!(json["txid"].is_string());

    Ok(())
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Value,
) -> anyhow::Result<StatusCode> {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("X-Dev-User", user)
        .body(Body::from(serde_json::to_vec(&body)?))?;
    Ok(app.clone().oneshot(req).await?.status())
}

/// 無効化済みのユーザー (Slack から取り込んだ未連携のプレースホルダなど) には、@here とリアクションの通知を作らない
#[tokio::test]
async fn test_notifications_skip_deactivated_users() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state);

    let (alice, bob, placeholder) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    for (id, name) in [(&alice, "alice"), (&bob, "bob"), (&placeholder, "slack")] {
        let status = send(
            &app,
            "POST",
            "/api/users",
            id,
            json!({"id": id, "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()), "display_name": name}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }
    let channel = uuid::Uuid::new_v4().to_string();
    let status = send(
        &app,
        "POST",
        "/api/channels",
        &alice,
        json!({"id": channel, "channel_name": format!("ch-{}", uuid::Uuid::new_v4().simple()), "is_private": false}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    for uid in [&bob, &placeholder] {
        let status = send(
            &app,
            "POST",
            &format!("/api/channels/{channel}/users"),
            &alice,
            json!({"user_id": uid}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }
    // プレースホルダの投稿 (取り込みと同じく、無効化する前に入れておく)
    let placeholder_message = uuid::Uuid::new_v4().to_string();
    let status = send(
        &app,
        "POST",
        "/api/messages",
        &placeholder,
        json!({"id": placeholder_message, "channel_id": channel, "content": "from slack"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("UPDATE users SET guest_kind='deactivated' WHERE id=$1")
            .bind(placeholder.as_str())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
    }

    let here = uuid::Uuid::new_v4().to_string();
    let status = send(
        &app,
        "POST",
        "/api/messages",
        &alice,
        json!({"id": here, "channel_id": channel, "content": "@here hi"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let status = send(
        &app,
        "POST",
        &format!("/api/messages/{placeholder_message}/reactions"),
        &alice,
        json!({"emoji": "👍"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let check = db.clone();
    let count = |user: String, message: String| {
        let mut check = check.clone();
        async move {
            Notification::filter(
                Notification::fields()
                    .user_id()
                    .eq(user.as_str())
                    .and(Notification::fields().message_id().eq(message.as_str())),
            )
            .exec(&mut check)
            .await
            .map(|n| n.len())
        }
    };
    assert_eq!(count(bob.clone(), here.clone()).await?, 1);
    assert_eq!(count(placeholder.clone(), here.clone()).await?, 0);
    assert_eq!(
        count(placeholder.clone(), placeholder_message.clone()).await?,
        0
    );
    Ok(())
}

// ---- #620: チャンネルごとの通知レベル ----

async fn post_json(
    app: &Router,
    user: &str,
    method: &str,
    uri: &str,
    payload: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("X-Dev-User", user)
                .body(Body::from(payload.to_string()))?,
        )
        .await?;
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn create_user(app: &Router, name: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (status, _) = post_json(
        app,
        &id,
        "POST",
        "/api/users",
        json!({"id": id, "user_name": name, "display_name": name}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(id)
}

async fn new_channel(app: &Router, owner: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (status, _) = post_json(
        app,
        owner,
        "POST",
        "/api/channels",
        json!({"id": id, "channel_name": id, "is_private": false}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(id)
}

async fn join_channel(app: &Router, user: &str, channel: &str) -> anyhow::Result<()> {
    let (status, _) = post_json(
        &app,
        user,
        "POST",
        &format!("/api/channels/{channel}/join"),
        json!({}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

async fn set_level(app: &Router, user: &str, channel: &str, level: &str) -> anyhow::Result<()> {
    let (status, _) = post_json(
        &app,
        user,
        "PATCH",
        &format!("/api/channels/{channel}/notification-settings"),
        json!({"level": level}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

async fn send_message(
    app: &Router,
    user: &str,
    channel: &str,
    id: &str,
    content: &str,
    parent: Option<&str>,
) -> anyhow::Result<()> {
    let mut payload = json!({"id": id, "channel_id": channel, "content": content});
    if let Some(parent) = parent {
        payload["parent_id"] = json!(parent);
    }
    let (status, _) = post_json(app, user, "POST", "/api/messages", payload).await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

async fn notice_types(
    db: &mut toasty::Db,
    user: &str,
    message: &str,
) -> anyhow::Result<Vec<String>> {
    Ok(Notification::filter(
        Notification::fields()
            .user_id()
            .eq(user)
            .and(Notification::fields().message_id().eq(message)),
    )
    .exec(db)
    .await?
    .into_iter()
    .map(|n| n.notification_type)
    .collect())
}

#[tokio::test]
async fn test_notification_levels_all_mentions_mute() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let sender = create_user(&app, &format!("sender-{}", uuid::Uuid::new_v4().simple())).await?;
    let all = create_user(&app, &format!("all-{}", uuid::Uuid::new_v4().simple())).await?;
    let mentions =
        create_user(&app, &format!("mentions-{}", uuid::Uuid::new_v4().simple())).await?;
    let mute_name = format!("mute-{}", uuid::Uuid::new_v4().simple());
    let mute = create_user(&app, &mute_name).await?;
    let all_mentioned_name = format!("allm-{}", uuid::Uuid::new_v4().simple());
    let all_mentioned = create_user(&app, &all_mentioned_name).await?;
    let channel = new_channel(&app, &sender).await?;
    for user in [&all, &mentions, &mute, &all_mentioned] {
        join_channel(&app, user, &channel).await?;
    }
    set_level(&app, &all, &channel, "all").await?;
    set_level(&app, &mute, &channel, "mute").await?;
    set_level(&app, &all_mentioned, &channel, "all").await?;

    // 通常投稿: all だけに channel_message。mentions / mute には何も届かない。
    let plain = uuid::Uuid::new_v4().to_string();
    send_message(&app, &sender, &channel, &plain, "一般投稿", None).await?;
    let mut check_db = db.clone();
    assert_eq!(
        notice_types(&mut check_db, &all, &plain).await?,
        ["channel_message"]
    );
    assert!(
        notice_types(&mut check_db, &mentions, &plain)
            .await?
            .is_empty()
    );
    assert!(notice_types(&mut check_db, &mute, &plain).await?.is_empty());
    assert_eq!(
        notice_types(&mut check_db, &all_mentioned, &plain).await?,
        ["channel_message"]
    );
    assert!(
        notice_types(&mut check_db, &sender, &plain)
            .await?
            .is_empty()
    );

    // 明示メンション: mute も含めて届き、all では channel_message より優先される。
    let mentioned = uuid::Uuid::new_v4().to_string();
    send_message(
        &app,
        &sender,
        &channel,
        &mentioned,
        &format!("@{mute_name} と @{all_mentioned_name} を見て"),
        None,
    )
    .await?;
    // 明示メンションは mute にも届く。all でもメンションが channel_message より優先される。
    assert_eq!(
        notice_types(&mut check_db, &mute, &mentioned).await?,
        ["mention"]
    );
    assert_eq!(
        notice_types(&mut check_db, &all_mentioned, &mentioned).await?,
        ["mention"]
    );
    assert_eq!(
        notice_types(&mut check_db, &all, &mentioned).await?,
        ["channel_message"]
    );
    assert!(
        notice_types(&mut check_db, &mentions, &mentioned)
            .await?
            .is_empty()
    );

    // リアクション: mute の投稿者には届かない。all / mentions には従来どおり届く。
    let reaction_target = uuid::Uuid::new_v4().to_string();
    send_message(
        &app,
        &mute,
        &channel,
        &reaction_target,
        "リアクション対象",
        None,
    )
    .await?;
    let (status, _) = post_json(
        &app,
        &sender,
        "POST",
        &format!("/api/messages/{reaction_target}/reactions"),
        json!({"emoji": "👍"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice_types(&mut check_db, &mute, &reaction_target)
            .await?
            .is_empty()
    );

    let mentions_msg = uuid::Uuid::new_v4().to_string();
    send_message(&app, &mentions, &channel, &mentions_msg, "返信対象", None).await?;
    let (status, _) = post_json(
        &app,
        &sender,
        "POST",
        &format!("/api/messages/{mentions_msg}/reactions"),
        json!({"emoji": "🎉"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        notice_types(&mut check_db, &mentions, &mentions_msg).await?,
        ["reaction"]
    );
    Ok(())
}

#[tokio::test]
async fn test_notification_levels_thread_participants_and_left_thread() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let author = create_user(&app, &format!("author-{}", uuid::Uuid::new_v4().simple())).await?;
    let sender = create_user(&app, &format!("replyer-{}", uuid::Uuid::new_v4().simple())).await?;
    let all = create_user(&app, &format!("tall-{}", uuid::Uuid::new_v4().simple())).await?;
    let mentions = create_user(
        &app,
        &format!("tmentions-{}", uuid::Uuid::new_v4().simple()),
    )
    .await?;
    let mute = create_user(&app, &format!("tmute-{}", uuid::Uuid::new_v4().simple())).await?;
    let left = create_user(&app, &format!("tleft-{}", uuid::Uuid::new_v4().simple())).await?;
    let channel = new_channel(&app, &author).await?;
    for user in [&sender, &all, &mentions, &mute, &left] {
        join_channel(&app, user, &channel).await?;
    }
    set_level(&app, &all, &channel, "all").await?;
    set_level(&app, &mute, &channel, "mute").await?;
    // 退出済み参加者は default の mentions にして、返信が届かないことを確認する。
    // (all の参加者にはチャンネル投稿として channel_message が届くのが正しい挙動。)

    let root = uuid::Uuid::new_v4().to_string();
    send_message(&app, &author, &channel, &root, "スレッドの親", None).await?;
    for user in [&sender, &all, &mentions, &mute, &left] {
        let (status, _) = post_json(
            &app,
            user,
            "POST",
            &format!("/api/threads/{root}/join"),
            json!({}),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = post_json(
        &app,
        &left,
        "POST",
        &format!("/api/threads/{root}/leave"),
        json!({}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    let reply = uuid::Uuid::new_v4().to_string();
    send_message(
        &app,
        &sender,
        &channel,
        &reply,
        "参加者への返信",
        Some(&root),
    )
    .await?;
    let mut check_db = db.clone();
    // 親投稿者・参加中の all / mentions には reply。mute と退出済みには届かない。
    assert_eq!(
        notice_types(&mut check_db, &author, &reply).await?,
        ["reply"]
    );
    assert_eq!(notice_types(&mut check_db, &all, &reply).await?, ["reply"]);
    assert_eq!(
        notice_types(&mut check_db, &mentions, &reply).await?,
        ["reply"]
    );
    assert!(notice_types(&mut check_db, &mute, &reply).await?.is_empty());
    assert!(notice_types(&mut check_db, &left, &reply).await?.is_empty());
    assert!(
        notice_types(&mut check_db, &sender, &reply)
            .await?
            .is_empty()
    );
    Ok(())
}

/// Home の「返信への返信」でも、スレッドのルートに紐づく参加者へ reply が届く (#620)。
#[tokio::test]
async fn test_home_reply_to_reply_notifies_thread_participants() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let author = create_user(&app, &format!("hroot-{}", uuid::Uuid::new_v4().simple())).await?;
    let participant =
        create_user(&app, &format!("hpart-{}", uuid::Uuid::new_v4().simple())).await?;
    let target_author =
        create_user(&app, &format!("htarget-{}", uuid::Uuid::new_v4().simple())).await?;
    let actor = create_user(&app, &format!("hactor-{}", uuid::Uuid::new_v4().simple())).await?;
    let channel = new_channel(&app, &author).await?;
    for user in [&participant, &target_author, &actor] {
        join_channel(&app, user, &channel).await?;
    }

    let (status, root) = post_json(
        &app,
        &author,
        "POST",
        "/api/home",
        json!({"channel_id": channel, "content": "ルート"}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let root_id = root["id"].as_str().unwrap().to_string();

    let (status, _) = post_json(
        &app,
        &participant,
        "POST",
        "/api/home",
        json!({"in_reply_to": root_id, "content": "参加"}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (status, target_reply) = post_json(
        &app,
        &target_author,
        "POST",
        "/api/home",
        json!({"in_reply_to": root_id, "content": "返信先"}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let target_reply_id = target_reply["id"].as_str().unwrap().to_string();

    let (status, nested) = post_json(
        &app,
        &actor,
        "POST",
        "/api/home",
        json!({"in_reply_to": target_reply_id, "content": "返信への返信"}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let nested_id = nested["id"].as_str().unwrap().to_string();

    let mut check_db = db.clone();
    // 直接の返信先だけでなく、ルートに参加している別のメンバーにも届く。
    assert_eq!(
        notice_types(&mut check_db, &target_author, &nested_id).await?,
        ["reply"]
    );
    assert_eq!(
        notice_types(&mut check_db, &participant, &nested_id).await?,
        ["reply"]
    );
    Ok(())
}
