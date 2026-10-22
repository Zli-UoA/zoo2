use axum::{
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
