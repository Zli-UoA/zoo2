use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::Value;
use tower::ServiceExt;

async fn request(
    app: &Router,
    actor: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", actor)
        .header("content-type", "application/json")
        .body(match body {
            Some(body) => Body::from(serde_json::to_vec(&body)?),
            None => Body::empty(),
        })?;
    let response = app.clone().oneshot(req).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let json_val = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Ok((status, json_val))
}

#[tokio::test]
async fn test_calls_routes_authorization_and_lifecycle() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let outsider = uuid::Uuid::new_v4().to_string();
    let public_ch = uuid::Uuid::new_v4().to_string();
    let private_ch = uuid::Uuid::new_v4().to_string();
    let archived_ch = uuid::Uuid::new_v4().to_string();

    let mut tx = db.transaction().await?;
    for (id, name) in [(&alice, "Alice"), (&bob, "Bob"), (&outsider, "Outsider")] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(name)
        .exec(&mut tx)
        .await?;
    }

    // Public channel
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
    )
    .bind(public_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;

    // Private channel (alice & bob members)
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, true)",
    )
    .bind(private_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    for member in [&alice, &bob] {
        toasty::sql::statement(
            "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP)",
        )
        .bind(member.as_str())
        .bind(private_ch.as_str())
        .exec(&mut tx)
        .await?;
    }

    // Archived channel
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private, archive_at) VALUES ($1, $1, $2, false, CURRENT_TIMESTAMP)",
    )
    .bind(archived_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // 1. Initial status: public channel call should be inactive
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{public_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);
    assert_eq!(body["participant_count"], 0);
    assert_eq!(body["max_participants"], 6);
    assert!(body["ice_servers"].is_array());

    // 2. Private channel: outsider should get 404
    let (status, _) = request(
        &app,
        &outsider,
        "GET",
        &format!("/api/channels/{private_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 3. Member of private channel gets 200
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{private_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);

    // 4. Archived channel: returns 403 Forbidden
    let (status, _) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{archived_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 5. Active calls summary
    let (status, body) = request(&app, &alice, "GET", "/api/calls/active", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());

    // 6. call_presence に直接セッションを登録して検証
    let session_alice = uuid::Uuid::new_v4();
    toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, $4, $5, false, '2026-01-01T00:00:00Z', statement_timestamp() + interval '30 seconds')",
    )
    .bind(public_ch.clone())
    .bind(session_alice.to_string())
    .bind(alice.clone())
    .bind("Alice".to_string())
    .bind("Alice".to_string())
    .exec(&mut db)
    .await?;

    // GET /api/channels/{id}/call で参加者が取得でき、session_id が含まれないことを検証 ([P3])
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{public_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], true);
    assert_eq!(body["participant_count"], 1);
    let p = &body["participants"][0];
    assert_eq!(p["user_id"], alice);
    assert!(p.get("session_id").is_none());

    // Active calls summary でも session_id が露出していないことを確認
    let (status, body) = request(&app, &alice, "GET", "/api/calls/active", None).await?;
    assert_eq!(status, StatusCode::OK);
    let first_ch = body
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_id"] == public_ch)
        .expect("channel should be in active list");
    assert_eq!(first_ch["participant_count"], 1);
    let first_participant = &first_ch["participants"][0];
    assert_eq!(first_participant["user_id"], alice);
    assert!(first_participant.get("session_id").is_none());

    // 他人 (bob) の session_id を騙った signal 送信は拒否 (403 Forbidden)
    let (status, _) = request(
        &app,
        &bob,
        "POST",
        &format!("/api/channels/{public_ch}/call/signal"),
        Some(serde_json::json!({
            "session_id": session_alice,
            "target_session_id": session_alice,
            "signal": { "type": "offer", "sdp": "test" }
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 他人 (bob) が他人の session_id を leave しようとすると拒否 (404 または 403)
    let (status, _) = request(
        &app,
        &bob,
        "POST",
        &format!("/api/channels/{public_ch}/call/leave"),
        Some(serde_json::json!({
            "session_id": session_alice
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 本人 (alice) による leave は成功 (200 OK)
    let (status, _) = request(
        &app,
        &alice,
        "POST",
        &format!("/api/channels/{public_ch}/call/leave"),
        Some(serde_json::json!({
            "session_id": session_alice
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    // ルームが空になったことを確認
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{public_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);
    assert_eq!(body["participant_count"], 0);

    Ok(())
}

#[tokio::test]
async fn test_multi_instance_call_signaling_and_broadcast() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;

    // 2つの独立した AppState (マルチレプリカ相当) を生成
    let state_pod1 = AppState::new(db.clone(), config.clone());
    let state_pod2 = AppState::new(db.clone(), config.clone());
    let app_pod2 = create_router(state_pod2.clone());

    let user_a = uuid::Uuid::new_v4().to_string();
    let user_b = uuid::Uuid::new_v4().to_string();
    let channel_id = uuid::Uuid::new_v4().to_string();

    let mut tx = db.transaction().await?;
    for (id, name) in [(&user_a, "UserA"), (&user_b, "UserB")] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(name)
        .exec(&mut tx)
        .await?;
    }
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
    )
    .bind(channel_id.as_str())
    .bind(user_a.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // LISTEN コネクションの確立を少し待機
    tokio::time::sleep(Duration::from_millis(500)).await;

    // User A は Pod 1 に接続 (ローカルセッション登録 + DB プレゼンス登録)
    let session_a = uuid::Uuid::new_v4();
    let (tx_a, mut rx_a) = tokio::sync::mpsc::channel(256);
    state_pod1
        .call_registry
        .register_local_session(session_a, channel_id.clone(), user_a.clone(), tx_a)
        .await;

    toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, $4, $5, false, '2026-01-01T00:00:00Z', statement_timestamp() + interval '30 seconds')",
    )
    .bind(channel_id.clone())
    .bind(session_a.to_string())
    .bind(user_a.clone())
    .bind("UserA".to_string())
    .bind("UserA".to_string())
    .exec(&mut db)
    .await?;

    // User B は Pod 2 に接続 (ローカルセッション登録 + DB プレゼンス登録)
    let session_b = uuid::Uuid::new_v4();
    let (tx_b, _rx_b) = tokio::sync::mpsc::channel(256);
    state_pod2
        .call_registry
        .register_local_session(session_b, channel_id.clone(), user_b.clone(), tx_b)
        .await;

    toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, $4, $5, false, '2026-01-01T00:00:00Z', statement_timestamp() + interval '30 seconds')",
    )
    .bind(channel_id.clone())
    .bind(session_b.to_string())
    .bind(user_b.clone())
    .bind("UserB".to_string())
    .bind("UserB".to_string())
    .exec(&mut db)
    .await?;

    // User B が Pod 2 に POST /signal (宛先: User A) を送信
    let (status, resp_body) = request(
        &app_pod2,
        &user_b,
        "POST",
        &format!("/api/channels/{channel_id}/call/signal"),
        Some(serde_json::json!({
            "session_id": session_b,
            "target_session_id": session_a,
            "signal": { "type": "offer", "sdp": "v=0-multi-instance-test" }
        })),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "response body: {:?}", resp_body);

    // Pod 1 にいる User A の rx_a が、PostgreSQL LISTEN/NOTIFY 経由でシグナルを受信できることを検証！
    let received_event = tokio::time::timeout(Duration::from_secs(3), rx_a.recv())
        .await
        .expect("timeout waiting for distributed signal on Pod 1")
        .expect("rx_a stream closed");

    match received_event {
        backend::routes::calls::CallServerEvent::Signal {
            from_session_id,
            signal,
        } => {
            assert_eq!(from_session_id, session_b);
            assert_eq!(signal["type"], "offer");
            assert_eq!(signal["sdp"], "v=0-multi-instance-test");
        }
        other => panic!("expected Signal event, got: {:?}", other.event_type()),
    }

    // User B が Pod 2 に POST /leave を送信
    let (status, _) = request(
        &app_pod2,
        &user_b,
        "POST",
        &format!("/api/channels/{channel_id}/call/leave"),
        Some(serde_json::json!({
            "session_id": session_b
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);

    // Pod 1 にいる User A に ParticipantLeft イベントが届くことを検証！
    let left_event = tokio::time::timeout(Duration::from_secs(3), rx_a.recv())
        .await
        .expect("timeout waiting for ParticipantLeft on Pod 1")
        .expect("rx_a stream closed");

    match left_event {
        backend::routes::calls::CallServerEvent::ParticipantLeft {
            session_id,
            user_id,
        } => {
            assert_eq!(session_id, session_b);
            assert_eq!(user_id, user_b);
        }
        other => panic!("expected ParticipantLeft event, got: {:?}", other.event_type()),
    }

    Ok(())
}

#[tokio::test]
async fn test_session_id_global_uniqueness_and_channel_isolation() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());

    let user_1 = uuid::Uuid::new_v4().to_string();
    let user_2 = uuid::Uuid::new_v4().to_string();
    let ch_1 = uuid::Uuid::new_v4().to_string();
    let ch_2 = uuid::Uuid::new_v4().to_string();

    let mut tx = db.transaction().await?;
    for (id, name) in [(&user_1, "User1"), (&user_2, "User2")] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(name)
        .exec(&mut tx)
        .await?;
    }
    for ch in [&ch_1, &ch_2] {
        toasty::sql::statement(
            "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
        )
        .bind(ch.as_str())
        .bind(user_1.as_str())
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;

    let shared_session_id = uuid::Uuid::new_v4();

    // チャンネル 1 に session_id を登録
    toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, 'U1', 'U1', false, '2026-01-01T00:00:00Z', statement_timestamp() + interval '30 seconds')",
    )
    .bind(ch_1.clone())
    .bind(shared_session_id.to_string())
    .bind(user_1.clone())
    .exec(&mut db)
    .await?;

    // 別チャンネル (チャンネル 2) で同じ session_id を登録しようとすると UNIQUE 制約違反で失敗することを確認 ([P1])
    let dup_res = toasty::sql::statement(
        "INSERT INTO call_presence (
            channel_id, session_id, user_id, user_name, display_name, is_muted, joined_at, expires_at
         ) VALUES ($1, $2::uuid, $3, 'U2', 'U2', false, '2026-01-01T00:00:00Z', statement_timestamp() + interval '30 seconds')",
    )
    .bind(ch_2.clone())
    .bind(shared_session_id.to_string())
    .bind(user_2.clone())
    .exec(&mut db)
    .await;

    assert!(dup_res.is_err(), "Duplicate session_id across channels must fail UNIQUE constraint");

    // シグナルのチャンネル照合テスト ([P1]):
    // チャンネル 1 のセッション宛ての Signal イベントが、別チャンネルのセッションには配送されないことを検証
    let (tx1, mut rx1) = tokio::sync::mpsc::channel(256);
    let session_chan2 = uuid::Uuid::new_v4();
    state
        .call_registry
        .register_local_session(session_chan2, ch_2.clone(), user_2.clone(), tx1)
        .await;

    // チャンネル 1 宛ての Signal イベントを発行（target_session_id は session_chan2 を指定）
    let fake_event = backend::routes::calls::CallDistributedEvent::Signal {
        channel_id: ch_1.clone(),
        target_session_id: session_chan2,
        from_session_id: shared_session_id,
        signal: serde_json::json!({ "type": "offer", "sdp": "fake" }),
    };
    let payload = serde_json::to_string(&fake_event).unwrap();
    state.call_registry.dispatch_distributed_event(&payload).await;

    // チャンネルが一致しないため、rx1 にはイベントが配送されないことを確認
    let received = tokio::time::timeout(Duration::from_millis(200), rx1.recv()).await;
    assert!(received.is_err(), "Signal from different channel must not be delivered");

    Ok(())
}

