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
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn update(session: &str, content: &str) -> Value {
    json!({ "session_id": session, "content": content })
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

    let path = "/api/bubble-chat";

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

    // Content exceeding 1000 chars is rejected with 400 Bad Request.
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
    assert_eq!(
        request(
            &app_a,
            Some(&alice),
            path,
            Some(update(&session1, "期限切れテスト")),
        )
        .await?
        .0,
        StatusCode::OK
    );
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "UPDATE bubble_chat_presence SET expires_at = statement_timestamp() - interval '1 second' WHERE user_id = $1",
    )
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    let (_, list) = request(&app_b, Some(&bob), path, None).await?;
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["user_id"] == alice)
    );

    // Cleanup on next write removes expired row.
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
