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
    actor: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
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
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn update(session: &str, parent: Option<&str>, active: bool) -> Value {
    json!({ "session_id": session, "parent_id": parent, "active": active })
}

#[tokio::test]
async fn test_presence_across_independent_instances() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    // Separate connections and AppState values, rather than clones sharing memory.
    let mut db = init_db(&config.database_url).await?;
    let app_a = create_router(AppState::new(db.clone(), config.clone()));
    let app_b = create_router(AppState::new(
        init_db(&config.database_url).await?,
        config.clone(),
    ));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let outsider = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let root = uuid::Uuid::new_v4().to_string();
    let session1 = uuid::Uuid::new_v4().to_string();
    let session2 = uuid::Uuid::new_v4().to_string();
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
    for id in [&channel, &other] {
        toasty::sql::statement("INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, true)")
            .bind(id.as_str()).bind(alice.as_str()).exec(&mut tx).await?;
        for member in [&alice, &bob] {
            toasty::sql::statement("INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP)")
                .bind(member.as_str()).bind(id.as_str()).exec(&mut tx).await?;
        }
    }
    toasty::sql::statement(
        "INSERT INTO messages (id, channel_id, user_id, content) VALUES ($1, $2, $3, 'root')",
    )
    .bind(root.as_str())
    .bind(channel.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    let path = format!("/api/channels/{channel}/typing");
    let thread = format!("{path}?parent_id={root}");
    let expected = json!([{ "id": alice, "display_name": "Alice" }]);

    // A writes, B reads. Concurrent upserts of the same session are idempotent.
    let (a, b) = tokio::join!(
        request(&app_a, &alice, &path, Some(update(&session1, None, true))),
        request(&app_b, &alice, &path, Some(update(&session1, None, true)))
    );
    assert_eq!(a?.0, StatusCode::OK);
    assert_eq!(b?.0, StatusCode::OK);
    assert_eq!(
        request(&app_b, &bob, &path, None).await?,
        (StatusCode::OK, expected.clone())
    );
    assert_eq!(request(&app_a, &alice, &path, None).await?.1, json!([]));

    // A fresh instance (including after a restart) sees the same shared state.
    let app_c = create_router(AppState::new(init_db(&config.database_url).await?, config));
    assert_eq!(request(&app_c, &bob, &path, None).await?.1, expected);

    // Stopping one tab must not erase the other tab's presence, across instances.
    assert_eq!(
        request(&app_b, &alice, &path, Some(update(&session2, None, true)))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app_a, &alice, &path, Some(update(&session1, None, false)))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(request(&app_a, &bob, &path, None).await?.1, expected);
    assert_eq!(
        request(&app_b, &alice, &path, Some(update(&session2, None, false)))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(request(&app_a, &bob, &path, None).await?.1, json!([]));

    // Scope and permissions are checked on every instance, including thread roots.
    assert_eq!(
        request(
            &app_a,
            &alice,
            &path,
            Some(update(&session1, Some(&root), true))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(request(&app_b, &bob, &thread, None).await?.1, expected);
    assert_eq!(request(&app_b, &bob, &path, None).await?.1, json!([]));
    assert_eq!(
        request(&app_b, &outsider, &path, None).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app_b,
            &outsider,
            &path,
            Some(update(&session2, None, true))
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let wrong_thread = format!("/api/channels/{other}/typing?parent_id={root}");
    assert_eq!(
        request(&app_b, &bob, &wrong_thread, None).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app_b,
            &bob,
            &format!("/api/channels/{other}/typing"),
            Some(update(&session2, Some(&root), true))
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );

    let mut tx = db.transaction().await?;
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2")
        .bind(channel.as_str())
        .bind(alice.as_str())
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    assert_eq!(request(&app_b, &bob, &thread, None).await?.1, json!([]));
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP)")
        .bind(alice.as_str()).bind(channel.as_str()).exec(&mut tx).await?;
    tx.commit().await?;

    // Expiry uses database time, even if the originating instance never stops.
    let mut tx = db.transaction().await?;
    let rows = toasty::sql::query("SELECT (expires_at > statement_timestamp() AND expires_at <= statement_timestamp() + interval '6 seconds') FROM typing_presence WHERE channel_id = $1")
        .bind(channel.as_str()).exec(&mut tx).await?;
    assert_eq!(rows.len(), 1);
    let toasty::stmt::Value::Record(record) = &rows[0] else {
        panic!("expected record")
    };
    assert!(matches!(record.fields[0], toasty::stmt::Value::Bool(true)));
    toasty::sql::statement("UPDATE typing_presence SET expires_at = statement_timestamp() - interval '1 second' WHERE channel_id = $1")
        .bind(channel.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    assert_eq!(request(&app_b, &bob, &thread, None).await?.1, json!([]));
    assert_eq!(
        request(&app_b, &bob, &path, Some(update(&session2, None, false)))
            .await?
            .0,
        StatusCode::OK
    );
    let mut tx = db.transaction().await?;
    assert!(
        toasty::sql::query("SELECT session_id FROM typing_presence WHERE channel_id = $1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?
            .is_empty()
    );
    toasty::sql::statement("UPDATE channels SET archive_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    assert_eq!(
        request(&app_a, &alice, &path, Some(update(&session1, None, true)))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app_b, &bob, &path, None).await?.0,
        StatusCode::NOT_FOUND
    );

    // Remove only this test's fixtures.
    let mut tx = db.transaction().await?;
    toasty::sql::statement("DELETE FROM messages WHERE channel_id = $1")
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
    for id in [&channel, &other] {
        toasty::sql::statement("DELETE FROM channel_user WHERE channel_id = $1")
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("DELETE FROM channels WHERE id = $1")
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
    }
    for id in [&alice, &bob, &outsider] {
        toasty::sql::statement("DELETE FROM users WHERE id = $1")
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
