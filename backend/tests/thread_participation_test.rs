//! Uses the migrated PostgreSQL test database. No Electric server is needed.
#[path = "support/thread_db.rs"]
mod thread_db;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
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
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
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
    // The participant list reflects only people who are currently in the thread.
    let detail = ok(&app, &users[2], "GET", &path, Value::Null).await?;
    let listed: Vec<&str> = detail["data"]["participants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&users[2].as_str()));
    assert!(!listed.contains(&users[1].as_str()));
    assert_eq!(detail["data"]["participant_count"], listed.len());
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
    // Like a channel, replying needs membership: a user who left must join again first.
    assert_eq!(
        call(&app, &users[1], "POST", "/api/messages", nested.clone())
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    // Home replies into a channel thread are gated the same way and never rejoin silently.
    let home_root = ok(
        &app,
        &users[0],
        "POST",
        "/api/home",
        json!({"content":"home root","channel_id":channel,"visibility":"public"}),
    )
    .await?["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let home_reply = json!({"content":"via home","in_reply_to":home_root,"channel_id":channel,"visibility":"public"});
    ok(&app, &users[1], "POST", "/api/home", home_reply.clone()).await?;
    let home_path = format!("/api/threads/{home_root}");
    ok(
        &app,
        &users[1],
        "POST",
        &format!("{home_path}/leave"),
        json!({}),
    )
    .await?;
    assert_eq!(
        call(&app, &users[1], "POST", "/api/home", home_reply)
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ok(&app, &users[1], "GET", &home_path, Value::Null).await?["data"]["membership"],
        "left"
    );
    assert_eq!(
        ok(&app, &users[1], "GET", &path, Value::Null).await?["data"]["tracked"],
        true
    );
    ok(
        &app,
        &users[1],
        "POST",
        &format!("{path}/unarchive"),
        json!({}),
    )
    .await?;
    ok(&app, &users[1], "POST", &format!("{path}/join"), json!({})).await?;
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
    fixture.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn rollout_http_keeps_legacy_threads_readable_and_isolates_phases() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let mut db = init_db(&fixture.config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), fixture.config.clone()));
    let author = uuid::Uuid::new_v4().to_string();
    let reader = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let child = uuid::Uuid::new_v4().to_string();
    let nested = uuid::Uuid::new_v4().to_string();
    for user in [&author, &reader] {
        ok(
            &app,
            user,
            "POST",
            "/api/users",
            json!({"id":user,"user_name":user,"display_name":user}),
        )
        .await?;
    }
    ok(
        &app,
        &author,
        "POST",
        "/api/channels",
        json!({"id":channel,"channel_name":channel,"is_private":false}),
    )
    .await?;
    ok(
        &app,
        &reader,
        "POST",
        &format!("/api/channels/{channel}/join"),
        json!({}),
    )
    .await?;
    let root = ok(
        &app,
        &author,
        "POST",
        "/api/home",
        json!({"channel_id":channel,"content":"Root"}),
    )
    .await?["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (id, parent) in [(&child, Some(&root)), (&nested, Some(&child))] {
        ok(
            &app,
            &author,
            "POST",
            "/api/messages",
            json!({"id":id,"channel_id":channel,"parent_id":parent,"content":"Rollout"}),
        )
        .await?;
    }
    let path = format!("/api/threads/{child}");
    for phase in ["collecting", "normalizing"] {
        toasty::sql::statement("UPDATE thread_rollout SET phase=$1")
            .bind(phase)
            .exec(&mut db)
            .await?;
        let index = ok(&app, &reader, "GET", "/api/threads", Value::Null).await?;
        assert!(
            index["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == root)
        );
        assert_eq!(
            ok(&app, &reader, "GET", &path, Value::Null).await?["data"]["id"],
            child
        );
        ok(&app, &reader, "POST", &format!("{path}/join"), json!({})).await?;
        ok(&app, &reader, "POST", &format!("{path}/leave"), json!({})).await?;
    }
    toasty::sql::statement("SELECT migrate_thread_batch(1)")
        .exec(&mut db)
        .await?;
    let migrated = ok(&app, &reader, "GET", &path, Value::Null).await?;
    assert_eq!(migrated["data"]["id"], root);
    assert_eq!(migrated["data"]["membership"], "left");
    toasty::sql::statement("UPDATE thread_rollout SET phase='ready'")
        .exec(&mut db)
        .await?;
    assert!(
        ok(&app, &reader, "GET", "/api/threads", Value::Null).await?["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ok(&app, &reader, "POST", &format!("{path}/join"), json!({})).await?;
    assert_eq!(
        ok(&app, &reader, "GET", &path, Value::Null).await?["data"]["membership"],
        "joined"
    );

    // A target after the first 500 replies must still be in the Home response.
    toasty::sql::statement("INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) SELECT md5($1||n::text),$2,$3,'history',$1,'2020-01-01'::timestamp+n*interval '1 second' FROM generate_series(1,501) n")
        .bind(root.as_str()).bind(author.as_str()).bind(channel.as_str()).exec(&mut db).await?;
    let page = ok(
        &app,
        &reader,
        "GET",
        &format!("/api/home/{nested}/thread"),
        Value::Null,
    )
    .await?;
    let posts = page.as_array().unwrap();
    assert_eq!(posts.len(), 500);
    assert!(posts.iter().any(|v| v["id"] == nested));
    assert!(posts.iter().any(|v| v["id"] == root));

    // Replying to a surviving child of a deleted root is a clear API 404.
    toasty::sql::statement("UPDATE messages SET deleted_at=now() WHERE id=$1")
        .bind(root.as_str())
        .exec(&mut db)
        .await?;
    let (status, _) = call(&app, &author, "POST", "/api/messages", json!({"id":uuid::Uuid::new_v4().to_string(),"channel_id":channel,"parent_id":child,"content":"Reply"})).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        &author,
        "POST",
        "/api/home",
        json!({"in_reply_to":child,"content":"Home reply"}),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    fixture.cleanup().await?;
    Ok(())
}
