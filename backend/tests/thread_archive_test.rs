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
async fn nested_threads_archive_and_home_replies_reopen_them() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    request(
        &app,
        "POST",
        &user,
        "/api/users",
        json!({"id":user,"user_name":user,"display_name":"Thread tester"}),
    )
    .await?;
    request(
        &app,
        "POST",
        &user,
        "/api/channels",
        json!({"id":channel,"channel_name":channel,"is_private":false}),
    )
    .await?;
    // Start from Home so this test does not depend on the workspace's LTL pattern.
    let root = request(
        &app,
        "POST",
        &user,
        "/api/home",
        json!({"channel_id":channel,"content":"Root"}),
    )
    .await?;
    let root_id = root["id"].as_str().unwrap();
    let child = request(
        &app,
        "POST",
        &user,
        "/api/messages",
        json!({"channel_id":channel,"content":"Child","parent_id":root_id}),
    )
    .await?;
    let child_id = child["data"]["id"].as_str().unwrap();
    request(
        &app,
        "POST",
        &user,
        "/api/messages",
        json!({"channel_id":channel,"content":"Grandchild","parent_id":child_id}),
    )
    .await?;
    toasty::sql::statement("UPDATE messages SET created_at=(now() AT TIME ZONE 'UTC')-interval '2 days',thread_activity_at=(now() AT TIME ZONE 'UTC')-interval '2 days' WHERE channel_id=$1")
        .bind(channel.as_str()).exec(&mut db).await?;
    archive_idle_threads(&db).await?;
    let threads = request(
        &app,
        "GET",
        &user,
        "/api/threads?archived=true",
        Value::Null,
    )
    .await?;
    for id in [root_id, child_id] {
        let entry = threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == id)
            .unwrap();
        assert!(
            entry["thread_archive_at"].is_string(),
            "nested threads must also archive"
        );
    }
    let reopened = request(
        &app,
        "POST",
        &user,
        &format!("/api/threads/{child_id}/unarchive"),
        json!({}),
    )
    .await?;
    assert!(reopened["data"]["thread_archive_at"].is_null());
    let unchanged = request(
        &app,
        "POST",
        &user,
        &format!("/api/threads/{child_id}/unarchive"),
        json!({}),
    )
    .await?;
    assert!(
        unchanged["txid"].is_null(),
        "an unchanged thread must not allocate a replication txid"
    );

    toasty::sql::statement("UPDATE messages SET thread_activity_at=(now() AT TIME ZONE 'UTC')-interval '23 hours' WHERE id=$1")
        .bind(child_id).exec(&mut db).await?;
    archive_idle_threads(&db).await?;
    // A thread idle for less than one day stays active even if its replies are old.
    let threads = request(&app, "GET", &user, "/api/threads", Value::Null).await?;
    assert!(
        threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == child_id)
            .unwrap()["thread_archive_at"]
            .is_null()
    );
    // Home writes through its own synchronization trigger, bypassing create_message_impl.
    request(
        &app,
        "POST",
        &user,
        "/api/home",
        json!({"content":"Home reply","in_reply_to":root_id}),
    )
    .await?;
    let threads = request(&app, "GET", &user, "/api/threads", Value::Null).await?;
    assert!(
        threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == root_id)
            .unwrap()["thread_archive_at"]
            .is_null()
    );

    // A normal reply can be timestamped before a concurrent archive and inserted after it.
    let mut pending_db = db.clone();
    let mut pending = pending_db.transaction().await?;
    let started = backend::apps::one(
        &mut pending,
        "SELECT json_build_object('at',date_trunc('milliseconds',now() AT TIME ZONE 'UTC'))::text",
        &[],
    )
    .await?;
    let started = started["at"].as_str().unwrap();
    toasty::sql::statement("UPDATE messages SET thread_archive_at=GREATEST(now() AT TIME ZONE 'UTC',$2::timestamp+interval '1 millisecond') WHERE id=$1")
        .bind(root_id).bind(started).exec(&mut db).await?;
    toasty::sql::statement("INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) VALUES($1,$2,$3,'Concurrent reply',$4,$5::timestamp)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(user.as_str()).bind(channel.as_str()).bind(root_id).bind(started).exec(&mut pending).await?;
    pending.commit().await?;
    let threads = request(&app, "GET", &user, "/api/threads", Value::Null).await?;
    assert!(
        threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == root_id)
            .unwrap()["thread_archive_at"]
            .is_null()
    );

    // A historical import must not reopen an archived thread.
    toasty::sql::statement(
        "UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1",
    )
    .bind(root_id)
    .exec(&mut db)
    .await?;
    toasty::sql::statement("INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) VALUES($1,$2,$3,'Historical import',$4,(now() AT TIME ZONE 'UTC')-interval '30 days')")
        .bind(uuid::Uuid::new_v4().to_string()).bind(user.as_str()).bind(channel.as_str()).bind(root_id).exec(&mut db).await?;
    let threads = request(
        &app,
        "GET",
        &user,
        "/api/threads?archived=true",
        Value::Null,
    )
    .await?;
    assert!(
        threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == root_id)
            .unwrap()["thread_archive_at"]
            .is_string()
    );

    // The same endpoint must not disclose private threads to nonmembers.
    toasty::sql::statement("UPDATE channels SET is_private=true WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    let outsider = uuid::Uuid::new_v4().to_string();
    request(
        &app,
        "POST",
        &outsider,
        "/api/users",
        json!({"id":outsider,"user_name":outsider,"display_name":"Outsider"}),
    )
    .await?;
    let threads = request(&app, "GET", &outsider, "/api/threads", Value::Null).await?;
    assert!(
        !threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["channel_id"] == channel)
    );
    let path = format!("/api/threads/{root_id}/unarchive");
    assert_eq!(
        response(&app, "POST", &outsider, &path, json!({})).await?.0,
        StatusCode::NOT_FOUND
    );
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
    )
    .bind(outsider.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    toasty::sql::statement("UPDATE channels SET posting_policy='managers' WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        response(&app, "POST", &outsider, &path, json!({})).await?.0,
        StatusCode::FORBIDDEN
    );
    toasty::sql::statement("UPDATE channels SET archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        response(&app, "POST", &user, &path, json!({})).await?.0,
        StatusCode::NOT_FOUND
    );
    toasty::sql::statement("UPDATE channels SET archive_at=NULL WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        response(
            &app,
            "POST",
            &user,
            "/api/threads/not-an-id/unarchive",
            json!({})
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    // Deleting all replies must not strand an already archived parent in read-only mode.
    toasty::sql::statement(
        "UPDATE messages SET deleted_at=now() AT TIME ZONE 'UTC' WHERE parent_id=$1",
    )
    .bind(root_id)
    .exec(&mut db)
    .await?;
    let reopened = request(
        &app,
        "POST",
        &user,
        &format!("/api/threads/{root_id}/unarchive"),
        json!({}),
    )
    .await?;
    assert!(reopened["data"]["thread_archive_at"].is_null());
    Ok(())
}

#[tokio::test]
async fn historical_import_does_not_reorder_existing_threads() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    request(
        &app,
        "POST",
        &user,
        "/api/users",
        json!({"id":user,"user_name":user,"display_name":"Thread order tester"}),
    )
    .await?;
    request(
        &app,
        "POST",
        &user,
        "/api/channels",
        json!({"id":channel,"channel_name":channel,"is_private":false}),
    )
    .await?;
    let mut roots = Vec::new();
    // Parallel archive tests sweep all channels, so keep these activities within one day.
    for hours in [1, 2] {
        let root = request(
            &app,
            "POST",
            &user,
            "/api/home",
            json!({"channel_id":channel,"content":format!("Thread {hours}")}),
        )
        .await?;
        let root_id = root["id"].as_str().unwrap().to_owned();
        let reply = request(
            &app,
            "POST",
            &user,
            "/api/messages",
            json!({"channel_id":channel,"content":"Existing reply","parent_id":root_id}),
        )
        .await?;
        toasty::sql::statement("UPDATE messages SET created_at=(now() AT TIME ZONE 'UTC')-($2::int * interval '1 hour') WHERE id=$1")
            .bind(reply["data"]["id"].as_str().unwrap()).bind(hours.to_string()).exec(&mut db).await?;
        // The migration backfills the latest existing reply before indexed reads start.
        toasty::sql::statement("UPDATE messages SET created_at=(now() AT TIME ZONE 'UTC')-interval '30 days',thread_activity_at=(now() AT TIME ZONE 'UTC')-($2::int * interval '1 hour') WHERE id=$1")
            .bind(root_id.as_str()).bind(hours.to_string()).exec(&mut db).await?;
        roots.push(root_id);
    }
    for imported in [false, true] {
        if imported {
            toasty::sql::statement("INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) VALUES($1,$2,$3,'Historical reply',$4,(now() AT TIME ZONE 'UTC')-interval '300 days')")
                .bind(uuid::Uuid::new_v4().to_string()).bind(user.as_str()).bind(channel.as_str()).bind(roots[0].as_str()).exec(&mut db).await?;
        }
        let threads = request(&app, "GET", &user, "/api/threads", Value::Null).await?;
        let order: Vec<_> = threads["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["channel_id"] == channel)
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(order, roots, "imported: {imported}");
    }
    Ok(())
}

#[tokio::test]
async fn thread_index_limits_membership_history_and_page_size() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    request(
        &app,
        "POST",
        &user,
        "/api/users",
        json!({"id":user,"user_name":user,"display_name":"Paging tester"}),
    )
    .await?;
    let mut channels = Vec::new();
    for _ in 0..2 {
        let channel = uuid::Uuid::new_v4().to_string();
        request(
            &app,
            "POST",
            &user,
            "/api/channels",
            json!({"id":channel,"channel_name":channel,"is_private":false}),
        )
        .await?;
        toasty::sql::statement("WITH roots AS (INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT gen_random_uuid()::text,$1,$2,'Root '||n,(now() AT TIME ZONE 'UTC')-interval '1 hour'+n*interval '1 second' FROM generate_series(1,105) n RETURNING id,user_id,channel_id,created_at) INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) SELECT gen_random_uuid()::text,user_id,channel_id,'Reply',id,created_at FROM roots")
            .bind(user.as_str()).bind(channel.as_str()).exec(&mut db).await?;
        channels.push(channel);
    }
    toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1 AND channel_id=$2")
        .bind(user.as_str())
        .bind(channels[1].as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE channel_id=$1 AND content='Root 1'")
        .bind(channels[0].as_str()).exec(&mut db).await?;
    let first = request(&app, "GET", &user, "/api/threads", Value::Null).await?;
    let items = first["items"].as_array().unwrap();
    assert_eq!(items.len(), 100);
    assert!(
        items
            .iter()
            .all(|row| row["channel_id"] == channels[0] && row["thread_archive_at"].is_null())
    );
    // A cursor remains usable after the row at the page boundary is deleted.
    toasty::sql::statement("UPDATE messages SET deleted_at=now() AT TIME ZONE 'UTC' WHERE id=$1")
        .bind(items.last().unwrap()["id"].as_str().unwrap())
        .exec(&mut db)
        .await?;
    let cursor = first["next_cursor"].as_str().unwrap();
    let query = format!(
        "before={}",
        percent_encoding::utf8_percent_encode(cursor, percent_encoding::NON_ALPHANUMERIC)
    );
    let second = request(
        &app,
        "GET",
        &user,
        &format!("/api/threads?{query}"),
        Value::Null,
    )
    .await?;
    assert_eq!(second["items"].as_array().unwrap().len(), 4);
    assert!(second["next_cursor"].is_null());
    for row in second["items"].as_array().unwrap() {
        assert!(!items.iter().any(|other| other["id"] == row["id"]));
    }
    let archive = request(
        &app,
        "GET",
        &user,
        &format!("/api/threads?archived=true&channel_id={}", channels[0]),
        Value::Null,
    )
    .await?;
    assert_eq!(archive["items"].as_array().unwrap().len(), 1);
    let small = request(&app, "GET", &user, "/api/threads?limit=2", Value::Null).await?;
    assert_eq!(small["items"].as_array().unwrap().len(), 2);
    let capped = request(&app, "GET", &user, "/api/threads?limit=1000", Value::Null).await?;
    assert_eq!(capped["items"].as_array().unwrap().len(), 100);
    assert_eq!(
        response(
            &app,
            "GET",
            &user,
            "/api/threads?before=invalid",
            Value::Null
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}

#[tokio::test]
async fn activity_migration_preserves_existing_replies_and_reopens() -> anyhow::Result<()> {
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
