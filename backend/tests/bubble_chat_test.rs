use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    app: &Router,
    actor: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(path)
        .header("content-type", "application/json");
    if let Some(actor) = actor {
        builder = builder.header("X-Dev-User", actor);
    }
    let req = builder.body(match body {
        Some(body) => Body::from(serde_json::to_vec(&body)?),
        None => Body::empty(),
    })?;
    let response = app.clone().oneshot(req).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, body))
}

fn current_time_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn update(session: &str, content: &str) -> Value {
    json!({ "session_id": session, "seq": 0, "content": content, "sent_at": current_time_ms() })
}

fn update_with_seq(session: &str, seq: i64, content: &str) -> Value {
    json!({ "session_id": session, "seq": seq, "content": content, "sent_at": current_time_ms() })
}

fn update_with_seq_and_sent_at(session: &str, seq: i64, content: &str, sent_at: i64) -> Value {
    json!({ "session_id": session, "seq": seq, "content": content, "sent_at": sent_at })
}

#[tokio::test]
async fn test_bubble_chat_presence_lifecycle() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;

    let mut db = init_db(&config.database_url).await?;
    let app_a = create_router(AppState::new(db.clone(), config.clone()));
    let app_b = create_router(AppState::new(
        init_db(&config.database_url).await?,
        config.clone(),
    ));

    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let session1 = uuid::Uuid::new_v4().to_string();
    let session2 = uuid::Uuid::new_v4().to_string();

    let mut tx = db.transaction().await?;
    for (id, display_name) in [(&alice, "Alice"), (&bob, "Bob")] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(display_name)
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;

    let path = "/api/bubble";

    // Supports both /api/bubble and /api/bubble-chat.
    assert_eq!(
        request(&app_a, Some(&alice), "/api/bubble-chat", None)
            .await?
            .0,
        StatusCode::OK
    );

    // Invalid dev user header is rejected with 400.
    assert_eq!(
        request(&app_a, Some("not-a-uuid"), path, None).await?.0,
        StatusCode::BAD_REQUEST
    );

    // A writes, B reads.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update(&session1, "こんにちは")),
        )
        .await?
        .0,
        StatusCode::OK
    );

    let (status, list) = request(&app_b, Some(&bob), path, None).await?;
    assert_eq!(status, StatusCode::OK);
    let alice_entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["user_id"] == alice)
        .expect("Alice bubble should be present");
    assert_eq!(alice_entry["display_name"], "Alice");
    assert_eq!(alice_entry["content"], "こんにちは");

    // Second tab updates with newer text; latest session wins for the same user.
    assert_eq!(
        request(
            &app_b,
            Some(&alice),
            path,
            Some(update(&session2, "別タブから入力中")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_a, Some(&bob), path, None).await?;
    let alice_entries: Vec<&Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["user_id"] == alice)
        .collect();
    assert_eq!(alice_entries.len(), 1);
    assert_eq!(alice_entries[0]["content"], "別タブから入力中");

    // Clearing session2 falls back to session1 which is still active.
    assert_eq!(
        request(&app_b, Some(&alice), path, Some(update(&session2, "   ")))
            .await?
            .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_a, Some(&bob), path, None).await?;
    let alice_entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["user_id"] == alice)
        .expect("Alice session1 should still be present");
    assert_eq!(alice_entry["content"], "こんにちは");

    // Clearing session1 removes Alice completely.
    assert_eq!(
        request(&app_a, Some(&alice), path, Some(update(&session1, "")))
            .await?
            .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["user_id"] == alice)
    );

    // Out-of-order sequence handling: older seq does not overwrite newer seq.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update_with_seq(&session1, 10, "最新の書き込み")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    // Delayed older write with seq=9 is ignored.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update_with_seq(&session1, 9, "遅延した古い書き込み")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    let alice_entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["user_id"] == alice)
        .expect("Alice bubble should be present");
    assert_eq!(alice_entry["content"], "最新の書き込み");

    // Newer clear with seq=11 clears presence.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update_with_seq(&session1, 11, "")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["user_id"] == alice)
    );

    // Stale older write with seq=10 arriving after seq=11 does not resurrect the row.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update_with_seq(&session1, 10, "復活試行")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["user_id"] == alice)
    );
    let too_long = "あ".repeat(1001);
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update(&session1, &too_long)),
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );

    // Expiry uses database time.
    // Verify valid presence first before testing expiration.
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update_with_seq(&session1, 12, "期限切れテスト")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    let alice_entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["user_id"] == alice)
        .expect("Alice bubble should be present before expiration");
    assert_eq!(alice_entry["content"], "期限切れテスト");

    // Advance expires_at into the past while retaining retention_until.
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "UPDATE bubble_chat_presence SET expires_at = statement_timestamp() - interval '1 second' WHERE user_id = $1",
    )
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // Presence disappears from list once expires_at has passed.
    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["user_id"] == alice)
    );

    // Stale write older than 5 minutes is rejected with 400 Bad Request.
    let now_ms = current_time_ms();
    let past_timestamp = now_ms - 600_000;
    let (status, _) = request(
        &app_a,
        Some(&alice),
        path,
        Some(update_with_seq_and_sent_at(
            &session1,
            13,
            "古い要求",
            past_timestamp,
        )),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Requests omitting seq or sent_at are rejected.
    let (status, _) = request(
        &app_a,
        Some(&alice),
        path,
        Some(json!({ "session_id": session1, "content": "seq欠落" })),
    )
    .await?;
    assert!(status.is_client_error());

    // Row is retained until retention_until expires.
    let mut tx = db.transaction().await?;
    assert!(
        !toasty::sql::query("SELECT session_id FROM bubble_chat_presence WHERE user_id = $1")
            .bind(alice.as_str())
            .exec(&mut tx)
            .await?
            .is_empty()
    );
    // Force retention_until into the past to verify cleanup.
    toasty::sql::statement(
        "UPDATE bubble_chat_presence SET retention_until = statement_timestamp() - interval '1 second' WHERE user_id = $1",
    )
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    // Cleanup on next write removes expired retention row.
    assert_eq!(
        request(&app_b, Some(&bob), path, Some(update(&session2, "")))
            .await?
            .0,
        StatusCode::OK
    );
    let mut tx = db.transaction().await?;
    assert!(
        toasty::sql::query("SELECT session_id FROM bubble_chat_presence WHERE user_id = $1")
            .bind(alice.as_str())
            .exec(&mut tx)
            .await?
            .is_empty()
    );
    for id in [&alice, &bob] {
        toasty::sql::statement("DELETE FROM users WHERE id = $1")
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;

    Ok(())
}
