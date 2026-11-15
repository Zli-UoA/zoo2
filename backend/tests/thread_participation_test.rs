//! Uses the migrated PostgreSQL test database. No Electric server is needed.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("X-Dev-User", user)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
async fn ok(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<Value> {
    let (status, value) = call(app, user, method, path, body).await?;
    anyhow::ensure!(status.is_success(), "{path}: {status} {value}");
    Ok(value)
}

#[tokio::test]
async fn participation_is_not_read_permission_and_leaves_survive_reopen() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    toasty::sql::statement("UPDATE thread_rollout SET phase='ready'")
        .exec(&mut db)
        .await?;
    let app = create_router(AppState::new(db.clone(), config));
    let users: Vec<_> = (0..4).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    for user in &users {
        ok(
            &app,
            user,
            "POST",
            "/api/users",
            json!({"id":user,"user_name":user,"display_name":user}),
        )
        .await?;
    }
    let channel = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &users[0],
        "POST",
        "/api/channels",
        json!({"id":channel,"channel_name":channel,"is_private":false}),
    )
    .await?;
    for user in &users[1..3] {
        ok(
            &app,
            user,
            "POST",
            &format!("/api/channels/{channel}/join"),
            json!({}),
        )
        .await?;
    }
    let root = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &users[0],
        "POST",
        "/api/messages",
        json!({"id":root,"channel_id":channel,"content":"Root"}),
    )
    .await?;
    let reply = uuid::Uuid::new_v4().to_string();
    let payload = json!({"id":reply,"channel_id":channel,"parent_id":root,"content":"First reply"});
    ok(&app, &users[1], "POST", "/api/messages", payload.clone()).await?;
    let path = format!("/api/threads/{root}");
    for user in &users[..2] {
        assert_eq!(
            ok(&app, user, "GET", &path, Value::Null).await?["data"]["membership"],
            "joined"
        );
    }
    // A third member and a public-channel reader can see it without participating.
    for user in &users[2..] {
        assert_eq!(
            ok(&app, user, "GET", &path, Value::Null).await?["data"]["membership"],
            "none"
        );
        let index = ok(&app, user, "GET", "/api/threads", Value::Null).await?;
        assert!(
            !index["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == root)
        );
    }
    ok(&app, &users[2], "POST", &format!("{path}/join"), json!({})).await?;
    let repeat = ok(&app, &users[2], "POST", &format!("{path}/join"), json!({})).await?;
    assert!(repeat["txid"].is_null());
    ok(&app, &users[1], "POST", &format!("{path}/leave"), json!({})).await?;
    // Retrying an already committed post must not undo the explicit leave.
    ok(&app, &users[1], "POST", "/api/messages", payload).await?;
    assert_eq!(
        ok(&app, &users[1], "GET", &path, Value::Null).await?["data"]["membership"],
        "left"
    );
    toasty::sql::statement(
        "UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1",
    )
    .bind(root.as_str())
    .exec(&mut db)
    .await?;
    for user in &users[..3] {
        let index = ok(&app, user, "GET", "/api/threads", Value::Null).await?;
        assert!(
            !index["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == root)
        );
    }
    ok(
        &app,
        &users[0],
        "POST",
        &format!("{path}/unarchive"),
        json!({}),
    )
    .await?;
    assert_eq!(
        ok(&app, &users[2], "GET", &path, Value::Null).await?["data"]["membership"],
        "joined"
    );
    assert_eq!(
        ok(&app, &users[1], "GET", &path, Value::Null).await?["data"]["membership"],
        "left"
    );
    toasty::sql::statement(
        "UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1",
    )
    .bind(root.as_str())
    .exec(&mut db)
    .await?;
    let nested_id = uuid::Uuid::new_v4().to_string();
    let nested =
        json!({"id":nested_id,"channel_id":channel,"parent_id":reply,"content":"Back again"});
    let created = ok(&app, &users[1], "POST", "/api/messages", nested.clone()).await?;
    assert_eq!(created["data"]["parent_id"], root);
    let state = ok(&app, &users[1], "GET", &path, Value::Null).await?;
    assert_eq!(state["data"]["membership"], "joined");
    assert!(state["data"]["thread_archive_at"].is_null());
    ok(&app, &users[1], "POST", &format!("{path}/leave"), json!({})).await?;
    ok(&app, &users[1], "POST", "/api/messages", nested).await?;
    assert_eq!(
        ok(&app, &users[1], "GET", &path, Value::Null).await?["data"]["membership"],
        "left"
    );
    // Old nested URLs resolve to the root without granting access to private channels.
    assert_eq!(
        ok(
            &app,
            &users[2],
            "GET",
            &format!("/api/threads/{reply}"),
            Value::Null
        )
        .await?["data"]["id"],
        root
    );
    toasty::sql::statement("UPDATE channels SET is_private=true WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        call(&app, &users[3], "GET", &path, Value::Null).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, &users[3], "POST", &format!("{path}/join"), json!({}))
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
