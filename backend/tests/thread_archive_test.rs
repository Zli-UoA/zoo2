//! Requires the migrated PostgreSQL test database.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    routes::{create_router, threads::archive_idle_threads},
    state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn response(
    app: &Router,
    method: &str,
    user: &str,
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
    if path.starts_with("/api/threads") {
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn request(
    app: &Router,
    method: &str,
    user: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<Value> {
    let (status, result) = response(app, method, user, path, body).await?;
    assert!(status.is_success(), "{status}: {result}");
    Ok(result)
}

#[tokio::test]
async fn idle_channel_threads_archive_and_a_reply_reopens_them() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    let parent = uuid::Uuid::new_v4().to_string();
    request(
        &app,
        "POST",
        &user,
        "/api/users",
        json!({"id":user,"user_name":format!("thread_archive_{}",uuid::Uuid::new_v4().simple()),"display_name":"Thread tester"}),
    )
    .await?;
    request(
        &app,
        "POST",
        &user,
        "/api/channels",
        json!({"id":parent,"channel_name":format!("thread-archive-{}",uuid::Uuid::new_v4().simple()),"is_private":false}),
    )
    .await?;
    let created = request(
        &app,
        "POST",
        &user,
        &format!("/api/channels/{parent}/threads"),
        json!({"title":"Idle thread","initial_post":"First post"}),
    )
    .await?;
    let thread = created["data"]["channel_id"].as_str().unwrap();
    let first_message = created["data"]["first_message_id"].as_str().unwrap();

    toasty::sql::statement(
        "UPDATE threads SET activity_at=(now() AT TIME ZONE 'UTC')-interval '2 days' WHERE channel_id=$1",
    )
    .bind(thread)
    .exec(&mut db)
    .await?;
    archive_idle_threads(&db).await?;
    let archived = request(
        &app,
        "GET",
        &user,
        &format!("/api/channels/{parent}/threads?archived=true"),
        Value::Null,
    )
    .await?;
    assert_eq!(archived["items"][0]["channel_id"], thread);
    assert!(archived["items"][0]["archived_at"].is_string());

    // The archive is independent from membership and does not block replies.
    request(
        &app,
        "POST",
        &user,
        "/api/messages",
        json!({"channel_id":thread,"content":"Still discussing","reply_to_message_id":first_message}),
    )
    .await?;
    let active = request(
        &app,
        "GET",
        &user,
        &format!("/api/channels/{parent}/threads?archived=false"),
        Value::Null,
    )
    .await?;
    assert_eq!(active["items"][0]["channel_id"], thread);
    assert!(active["items"][0]["archived_at"].is_null());
    Ok(())
}

#[tokio::test]
async fn legacy_thread_activity_backfill_preserves_archives_and_reopens() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let config = Config::from_env();
    let (mut client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let tx = client.transaction().await?;
    let schema = format!("thread_migration_{}", uuid::Uuid::new_v4().simple());
    tx.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema},public;
         CREATE TABLE messages (
           id text PRIMARY KEY,channel_id text,created_at timestamp(3),parent_id text,
           deleted_at timestamp(3),thread_activity_at timestamp(3),thread_archive_at timestamp(3)
         );
         INSERT INTO messages(id,channel_id,created_at,thread_activity_at,thread_archive_at) VALUES
           ('a','c1','2026-01-01',NULL,'2026-11-11'),
           ('b','c1','2026-01-01','2026-01-02',NULL),
           ('reopened','c1','2026-01-01','2026-11-11',NULL),
           ('empty','c1','2026-01-01',NULL,NULL);
         INSERT INTO messages(id,channel_id,created_at,parent_id,deleted_at) VALUES
           ('a-reply','c1','2026-11-10','a',NULL),
           ('b-reply','c1','2026-11-09','b',NULL),
           ('old-reply','c1','2026-11-08','reopened',NULL),
           ('deleted-reply','c1','2026-11-12','b','2026-11-12'),
           ('nested','c1','2026-11-11','a-reply',NULL);"
    ))
    .await?;
    tx.batch_execute(include_str!(
        "../../infra/db/migrations/20261119002000_thread_activity_index.sql"
    ))
    .await?;
    for (id, expected) in [
        ("a", "2026-11-10 00:00:00"),
        ("b", "2026-11-09 00:00:00"),
        ("reopened", "2026-11-11 00:00:00"),
        ("a-reply", "2026-11-11 00:00:00"),
    ] {
        let row = tx
            .query_one(
                "SELECT thread_activity_at::text FROM messages WHERE id=$1",
                &[&id],
            )
            .await?;
        assert_eq!(row.get::<_, String>(0), expected, "{id}");
    }
    assert!(
        tx.query_one(
            "SELECT thread_activity_at IS NULL FROM messages WHERE id='empty'",
            &[]
        )
        .await?
        .get::<_, bool>(0)
    );
    tx.batch_execute(
        "CREATE TRIGGER messages_thread_activity_insert AFTER INSERT ON messages
           FOR EACH ROW EXECUTE FUNCTION update_thread_activity();
         INSERT INTO messages(id,channel_id,created_at,parent_id) VALUES
           ('historical','c1','2026-01-02','a');",
    )
    .await?;
    let row = tx
        .query_one(
            "SELECT thread_activity_at::text,thread_archive_at::text FROM messages WHERE id='a'",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, String>(0), "2026-11-10 00:00:00");
    assert_eq!(row.get::<_, String>(1), "2026-11-11 00:00:00");
    tx.rollback().await?;
    drop(client);
    driver.abort();
    Ok(())
}
