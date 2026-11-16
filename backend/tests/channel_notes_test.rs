use anyhow::Context;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const E2EE_NOTES_MIGRATION: &str =
    include_str!("../../infra/db/migrations/20261128000003_e2ee_channel_notes.sql");

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

static CLEANUP_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", actor)
                .body(Body::from(if body.is_null() {
                    String::new()
                } else {
                    body.to_string()
                }))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn create_user(app: &Router, id: &str) -> String {
    let user_name = format!("note_{}", Uuid::new_v4().simple());
    assert_eq!(
        request(
            app,
            "POST",
            "/api/users",
            id,
            json!({"id":id,"user_name":user_name,"display_name":"Notes test"}),
        )
        .await
        .0,
        StatusCode::OK
    );
    user_name
}

async fn wait_until_blocked_by(
    observer: &tokio_postgres::Client,
    blocker_pid: i32,
) -> anyhow::Result<i32> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting = observer
                .query_opt(
                    "SELECT pid FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                    &[&blocker_pid],
                )
                .await?;
            if let Some(waiting) = waiting {
                break Ok::<i32, anyhow::Error>(waiting.get(0));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(error) => {
            let waiting = observer
                .query(
                    "SELECT format('%s|%s|%s|%s', pid, coalesce(wait_event_type, ''), query, pg_blocking_pids(pid)) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
                    &[],
                )
                .await?
                .into_iter()
                .map(|row| row.get::<_, String>(0))
                .collect::<Vec<_>>();
            anyhow::bail!(
                "timed out waiting for a session blocked by {blocker_pid}; current sessions: {waiting:?}; {error}"
            );
        }
    }
}

async fn wait_until_application_blocked_by(
    observer: &tokio_postgres::Client,
    application_name: &str,
    blocker_pid: i32,
) -> anyhow::Result<i32> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting = observer
                .query_opt(
                    "SELECT pid FROM pg_stat_activity WHERE application_name = $1 AND wait_event_type = 'Lock' AND $2 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                    &[&application_name, &blocker_pid],
                )
                .await?;
            if let Some(waiting) = waiting {
                break Ok::<i32, anyhow::Error>(waiting.get(0));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(error) => {
            let waiting = observer
                .query(
                    "SELECT format('%s|%s|%s|%s|%s', pid, application_name, coalesce(wait_event_type, ''), query, pg_blocking_pids(pid)) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
                    &[],
                )
                .await?
                .into_iter()
                .map(|row| row.get::<_, String>(0))
                .collect::<Vec<_>>();
            anyhow::bail!(
                "timed out waiting for application {application_name:?} to be blocked by {blocker_pid}; current sessions: {waiting:?}; {error}"
            );
        }
    }
}

fn database_url_with_application_name(database_url: &str, application_name: &str) -> String {
    format!(
        "{database_url}{}application_name={application_name}",
        if database_url.contains('?') { '&' } else { '?' }
    )
}

#[tokio::test]
async fn notes_hide_public_channels_from_nonmember_guests() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let owner = Uuid::new_v4().to_string();
    let guest = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &guest).await;
    toasty::sql::statement("UPDATE users SET guest_kind = 'multi' WHERE id = $1")
        .bind(guest.as_str())
        .exec(&mut db)
        .await?;
    let channel = Uuid::new_v4().to_string();
    let created = request(&app, "POST", "/api/channels", &owner, json!({
        "id": channel, "channel_name": format!("notes_{}", Uuid::new_v4().simple()), "is_private": false
    })).await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let notes = format!("/api/channels/{channel}/notes");
    let created_note = request(
        &app,
        "POST",
        &notes,
        &owner,
        json!({"title":"private from guests", "content":"content"}),
    )
    .await;
    assert_eq!(created_note.0, StatusCode::OK);
    let note_id = created_note.1["data"]["id"].as_str().unwrap();
    assert_eq!(
        request(&app, "GET", &notes, &guest, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("{notes}/{note_id}"),
            &guest,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn direct_sql_position_change_invalidates_an_in_flight_reorder() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel,
            "channel_name": format!("notes_direct_sql_race_{}", Uuid::new_v4().simple()),
            "is_private": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let collection = format!("/api/channels/{channel}/notes");
    let mut ids = Vec::new();
    for title in ["A", "B"] {
        let note = request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":title,"content":""}),
        )
        .await;
        assert_eq!(note.0, StatusCode::OK, "{}", note.1);
        ids.push(note.1["data"]["id"].as_str().unwrap().to_owned());
    }

    let (mut writer_connection, writer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(writer_driver);
    let writer_pid: i32 = writer_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let writer = writer_connection.transaction().await?;
    writer
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&channel],
        )
        .await?;
    writer
        .execute(
            "UPDATE channel_notes SET position = 2 WHERE id = $1",
            &[&ids[0]],
        )
        .await?;

    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let api = app.clone();
    let path = format!("{collection}/order");
    let actor = owner.clone();
    let reorder = tokio::spawn(async move {
        request(
            &api,
            "PATCH",
            &path,
            &actor,
            json!({"expected_note_ids":ids.clone(),"note_ids":[ids[1].clone(),ids[0].clone()]}),
        )
        .await
    });
    let reorder_pid = wait_until_blocked_by(&observer, writer_pid)
        .await
        .context("wait for reorder to serialize with direct SQL position change")?;
    let waiting_query: String = observer
        .query_one(
            "SELECT query FROM pg_stat_activity WHERE pid = $1",
            &[&reorder_pid],
        )
        .await?
        .get(0);
    assert!(
        waiting_query.contains("pg_advisory_xact_lock"),
        "reorder must wait for the common lock before reading positions, got: {waiting_query}"
    );
    writer.commit().await?;

    let (status, body) = reorder.await?;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "reorder must recheck expected IDs after direct SQL commits: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn channel_note_api_writes_and_cascade_are_independent_across_channels() -> anyhow::Result<()>
{
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let first = Uuid::new_v4().to_string();
    let second = Uuid::new_v4().to_string();
    for channel in [&first, &second] {
        let created = request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({
                "id": channel,
                "channel_name": format!("notes_independent_{}", Uuid::new_v4().simple()),
                "is_private": true
            }),
        )
        .await;
        assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    }

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&first],
        )
        .await?;
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);

    let first_app = app.clone();
    let first_owner = owner.clone();
    let first_id = first.clone();
    let blocked_write = tokio::spawn(async move {
        request(
            &first_app,
            "POST",
            &format!("/api/channels/{first_id}/notes"),
            &first_owner,
            json!({"title":"blocked","content":""}),
        )
        .await
    });
    wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("first channel API write waits on its channel lock")?;

    let (second_status, second_body) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request(
            &app,
            "POST",
            &format!("/api/channels/{second}/notes"),
            &owner,
            json!({"title":"independent","content":""}),
        ),
    )
    .await?;
    assert_eq!(second_status, StatusCode::OK, "{second_body}");

    blocker.commit().await?;
    let (first_status, first_body) = blocked_write.await?;
    assert_eq!(first_status, StatusCode::OK, "{first_body}");

    // A held note lock in one channel must not make a FK cascade in another
    // channel wait for a removed global note-write trigger.
    let note_lock = blocker_connection.transaction().await?;
    note_lock
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&first],
        )
        .await?;
    let (cascade_connection, cascade_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(cascade_driver);
    cascade_connection
        .execute("DELETE FROM channel_user WHERE channel_id = $1", &[&second])
        .await?;
    let deleted = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cascade_connection.execute("DELETE FROM channels WHERE id = $1", &[&second]),
    )
    .await??;
    assert_eq!(deleted, 1);
    let remaining: i64 = cascade_connection
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = $1",
            &[&second],
        )
        .await?
        .get(0);
    assert_eq!(remaining, 0, "channel cascade must delete its notes");
    note_lock.commit().await?;
    Ok(())
}

#[tokio::test]
async fn direct_sql_insert_delete_and_move_can_follow_the_note_lock_contract() -> anyhow::Result<()>
{
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let source = Uuid::new_v4().to_string();
    let destination = Uuid::new_v4().to_string();
    for channel in [&source, &destination] {
        let created = request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({
                "id": channel,
                "channel_name": format!("notes_write_lock_{}", Uuid::new_v4().simple()),
                "is_private": true
            }),
        )
        .await;
        assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    }
    let mut note_ids = Vec::new();
    for title in ["Delete", "Move"] {
        let note = request(
            &app,
            "POST",
            &format!("/api/channels/{source}/notes"),
            &owner,
            json!({"title":title,"content":""}),
        )
        .await;
        assert_eq!(note.0, StatusCode::OK, "{}", note.1);
        note_ids.push(note.1["data"]["id"].as_str().unwrap().to_owned());
    }

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);

    let insert_id = Uuid::new_v4().to_string();
    let insert_tx = blocker_connection.transaction().await?;
    insert_tx
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&source],
        )
        .await?;
    let (insert_connection, insert_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(insert_driver);
    let insert_owner = owner.clone();
    let insert_channel = source.clone();
    let insert_write = tokio::spawn(async move {
        let mut connection = insert_connection;
        let tx = connection.transaction().await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&insert_channel],
        )
        .await?;
        let inserted = tx
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ($1, $2, 'Direct', '', $3)",
                &[&insert_id, &insert_channel, &insert_owner],
            )
            .await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(inserted)
    });
    wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("direct INSERT waits before writing note rows")?;
    insert_tx.commit().await?;
    assert_eq!(insert_write.await??, 1);

    let delete_tx = blocker_connection.transaction().await?;
    delete_tx
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&source],
        )
        .await?;
    let (delete_connection, delete_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(delete_driver);
    let delete_id = note_ids[0].clone();
    let delete_channel = source.clone();
    let delete_write = tokio::spawn(async move {
        let mut connection = delete_connection;
        let tx = connection.transaction().await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&delete_channel],
        )
        .await?;
        let deleted = tx
            .execute("DELETE FROM channel_notes WHERE id = $1", &[&delete_id])
            .await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(deleted)
    });
    wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("direct DELETE waits before writing note rows")?;
    delete_tx.commit().await?;
    assert_eq!(delete_write.await??, 1);

    let move_tx = blocker_connection.transaction().await?;
    let mut lock_channels = vec![source.clone(), destination.clone()];
    lock_channels.sort();
    for channel in lock_channels {
        move_tx
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
                &[&channel],
            )
            .await?;
    }
    let (move_connection, move_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(move_driver);
    let move_id = note_ids[1].clone();
    let move_channel = destination.clone();
    let move_source = source.clone();
    let move_write = tokio::spawn(async move {
        let mut connection = move_connection;
        let tx = connection.transaction().await?;
        let mut channels = vec![move_source, move_channel.clone()];
        channels.sort();
        for channel in channels {
            tx.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
                &[&channel],
            )
            .await?;
        }
        let updated = tx
            .execute(
                "UPDATE channel_notes SET channel_id = $1, position = 0 WHERE id = $2",
                &[&move_channel, &move_id],
            )
            .await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(updated)
    });
    wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("direct UPDATE/move waits before writing note rows")?;
    move_tx.commit().await?;
    assert_eq!(move_write.await??, 1);
    Ok(())
}

#[tokio::test]
async fn notes_cannot_store_plaintext_in_encrypted_channels() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel, "channel_name": format!("notes_{}", Uuid::new_v4().simple()),
            "is_private": true, "encrypted": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let notes = format!("/api/channels/{channel}/notes");
    assert_eq!(
        request(&app, "GET", &notes, &owner, Value::Null).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &notes,
            &owner,
            json!({"title":"secret", "content":"plaintext"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}

#[tokio::test]
async fn encryption_rejects_active_and_restorable_deleted_notes() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;

    for deleted in [false, true] {
        let channel = Uuid::new_v4().to_string();
        let created = request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({
                "id": channel,
                "channel_name": format!("notes_encrypt_{}", Uuid::new_v4().simple()),
                "is_private": true
            }),
        )
        .await;
        assert_eq!(created.0, StatusCode::OK, "{}", created.1);
        let note = request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/notes"),
            &owner,
            json!({"title":"recoverable plaintext", "content":"private note"}),
        )
        .await;
        assert_eq!(note.0, StatusCode::OK, "{}", note.1);

        let (connection, driver) =
            tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        if deleted {
            connection
                .execute(
                    "UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '23 hours' WHERE id = $1",
                    &[&note.1["data"]["id"].as_str().unwrap()],
                )
                .await?;
        }
        let (status, body) = request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/encrypt"),
            &owner,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "E2EE_CONVERSION_BLOCKED", "{body}");
        assert_eq!(body["blockers"], json!(["channel_notes"]), "{body}");
        let encrypted: bool = connection
            .query_one("SELECT encrypted FROM channels WHERE id = $1", &[&channel])
            .await?
            .get(0);
        assert!(
            !encrypted,
            "failed conversion must leave channel unencrypted"
        );
    }

    let encrypted_channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": encrypted_channel,
            "channel_name": format!("notes_encrypted_{}", Uuid::new_v4().simple()),
            "is_private": true,
            "encrypted": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let (connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let insert = connection
        .execute(
            "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ($1, $2, 'plaintext', 'body', $3)",
            &[&Uuid::new_v4().to_string(), &encrypted_channel, &owner],
        )
        .await;
    let error = insert.expect_err("DB must reject plaintext note writes after encryption");
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    Ok(())
}

#[tokio::test]
async fn channel_note_creation_serializes_with_channel_encryption() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel,
            "channel_name": format!("notes_encrypt_race_{}", Uuid::new_v4().simple()),
            "is_private": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT id FROM channels WHERE id = $1 FOR UPDATE",
            &[&channel],
        )
        .await?;

    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let api = app.clone();
    let encrypt_path = format!("/api/channels/{channel}/encrypt");
    let encrypt_actor = owner.clone();
    let encrypt = tokio::spawn(async move {
        request(&api, "POST", &encrypt_path, &encrypt_actor, Value::Null).await
    });
    let encrypt_pid = wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("wait for encryption request to block on the channel")?;

    let api = app.clone();
    let path = format!("/api/channels/{channel}/notes");
    let actor = owner.clone();
    let pending = tokio::spawn(async move {
        request(
            &api,
            "POST",
            &path,
            &actor,
            json!({"title":"must not be stored", "content":"plaintext"}),
        )
        .await
    });
    wait_until_blocked_by(&observer, encrypt_pid)
        .await
        .context("wait for note request to queue behind the encryption transaction")?;
    blocker.commit().await?;
    let (status, body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), encrypt).await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), pending).await??;
    assert_eq!(response.0, StatusCode::BAD_REQUEST, "{}", response.1);

    let note_count: i64 = observer
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = $1",
            &[&channel],
        )
        .await?
        .get(0);
    assert_eq!(
        note_count, 0,
        "plaintext note must not be inserted after conversion"
    );

    let encrypted_channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": encrypted_channel,
            "channel_name": format!("notes_direct_race_{}", Uuid::new_v4().simple()),
            "is_private": true,
            "encrypted": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let direct_url = config.database_url.clone();
    let direct_channel_for_insert = encrypted_channel;
    let owner_for_insert = owner.clone();
    let insert = tokio::spawn(async move {
        let (connection, driver) =
            tokio_postgres::connect(&direct_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ($1, $2, 'plaintext', 'body', $3)",
                &[&Uuid::new_v4().to_string(), &direct_channel_for_insert, &owner_for_insert],
            )
            .await
    });
    let error = insert
        .await?
        .expect_err("the direct write should fail after conversion commits");
    assert_eq!(error.code().map(|code| code.code()), Some("23514"));
    Ok(())
}

#[tokio::test]
async fn api_update_and_direct_sql_update_do_not_deadlock() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let (status, created) = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"id":channel,"channel_name":format!("notes_deadlock_{}", Uuid::new_v4().simple()),"is_private":false}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let collection = format!("/api/channels/{channel}/notes");
    let (status, note) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"initial","content":"body"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    let note_id = note["data"]["id"].as_str().unwrap().to_owned();

    let (mut direct, direct_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(direct_driver);
    let direct_pid: i32 = direct
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let direct_tx = direct.transaction().await?;
    direct_tx
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&channel],
        )
        .await?;
    direct_tx
        .execute(
            "UPDATE channel_notes SET title = 'direct' WHERE id = $1",
            &[&note_id],
        )
        .await?;

    let api = app.clone();
    let actor = owner.clone();
    let path = format!("{collection}/{note_id}");
    let update = tokio::spawn(async move {
        request(
            &api,
            "PATCH",
            &path,
            &actor,
            json!({"title":"api","content":"body","version":1}),
        )
        .await
    });
    let observer = tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer.1);
    wait_until_blocked_by(&observer.0, direct_pid).await?;

    direct_tx.commit().await?;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), update).await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["title"], "api");
    Ok(())
}

#[tokio::test]
async fn concurrent_creates_by_different_members_get_distinct_positions() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = Uuid::new_v4().to_string();
    let first_member = Uuid::new_v4().to_string();
    let second_member = Uuid::new_v4().to_string();
    for user in [&owner, &first_member, &second_member] {
        create_user(&app, user).await;
    }
    let channel = Uuid::new_v4().to_string();
    let (status, body) = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"id":channel,"channel_name":format!("notes_create_race_{}", Uuid::new_v4().simple()),"is_private":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for member in [&first_member, &second_member] {
        assert_eq!(
            request(
                &app,
                "POST",
                &format!("/api/channels/{channel}/users"),
                &owner,
                json!({"user_id":member}),
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let notes = format!("/api/channels/{channel}/notes");
    for round in 0..20 {
        let (first, second) = tokio::join!(
            request(
                &app,
                "POST",
                &notes,
                &first_member,
                json!({"title":format!("A{round}"),"content":""})
            ),
            request(
                &app,
                "POST",
                &notes,
                &second_member,
                json!({"title":format!("B{round}"),"content":""})
            ),
        );
        assert_eq!(first.0, StatusCode::OK, "{}", first.1);
        assert_eq!(second.0, StatusCode::OK, "{}", second.1);
    }
    let mut position_db = db.clone();
    let rows = toasty::sql::query(
        "SELECT position FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL ORDER BY position",
    )
    .bind(channel.as_str())
    .exec(&mut position_db)
    .await?;
    let positions = rows
        .iter()
        .map(|row| match row {
            toasty::stmt::Value::Record(record) => match record.fields.first() {
                Some(toasty::stmt::Value::I32(value)) => i64::from(*value),
                other => panic!("unexpected position: {other:?}"),
            },
            other => panic!("unexpected result: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(positions, (0..40).collect::<Vec<_>>());
    Ok(())
}

#[tokio::test]
async fn concurrent_restore_create_and_reorder_delete_keep_active_order_consistent()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = Uuid::new_v4().to_string();
    let second_member = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &second_member).await;
    let channel = Uuid::new_v4().to_string();
    let (status, body) = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"id":channel,"channel_name":format!("notes_order_race_{}", Uuid::new_v4().simple()),"is_private":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/users"),
            &owner,
            json!({"user_id":second_member}),
        )
        .await
        .0,
        StatusCode::OK
    );
    let notes = format!("/api/channels/{channel}/notes");
    let item_a = request(
        &app,
        "POST",
        &notes,
        &owner,
        json!({"title":"A","content":""}),
    )
    .await
    .1["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let item_b = request(
        &app,
        "POST",
        &notes,
        &owner,
        json!({"title":"B","content":""}),
    )
    .await
    .1["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{notes}/{item_a}"),
            &owner,
            json!({"version":1}),
        )
        .await
        .0,
        StatusCode::OK
    );

    let restore_path = format!("{notes}/{item_a}/restore");
    let (restored, created) = tokio::join!(
        request(&app, "POST", &restore_path, &owner, Value::Null,),
        request(
            &app,
            "POST",
            &notes,
            &second_member,
            json!({"title":"C","content":""}),
        ),
    );
    assert_eq!(restored.0, StatusCode::OK, "{}", restored.1);
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let (status, list) = request(&app, "GET", &notes, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let ids = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 3);
    let order_path = format!("{notes}/order");
    let delete_path = format!("{notes}/{item_b}");
    let (reordered, deleted) = tokio::join!(
        request(
            &app,
            "PATCH",
            &order_path,
            &owner,
            json!({"expected_note_ids":ids.clone(),"note_ids":[item_b.clone(), created.1["data"]["id"].as_str().unwrap(), restored.1["data"]["id"].as_str().unwrap()]}),
        ),
        request(
            &app,
            "DELETE",
            &delete_path,
            &second_member,
            json!({"version":1}),
        ),
    );
    assert!(
        [StatusCode::OK, StatusCode::CONFLICT].contains(&reordered.0),
        "unexpected reorder response: {} {}",
        reordered.0,
        reordered.1
    );
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.1);

    let mut position_db = db;
    let rows = toasty::sql::query(
        "SELECT position FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL ORDER BY position",
    )
    .bind(channel.as_str())
    .exec(&mut position_db)
    .await?;
    let positions = rows
        .iter()
        .map(|row| match row {
            toasty::stmt::Value::Record(record) => match record.fields.first() {
                Some(toasty::stmt::Value::I32(value)) => *value,
                other => panic!("unexpected position: {other:?}"),
            },
            other => panic!("unexpected result: {other:?}"),
        })
        .collect::<Vec<_>>();
    let mut actual = positions.clone();
    actual.sort_unstable();
    actual.dedup();
    assert_eq!(
        actual.len(),
        positions.len(),
        "active positions must be unique"
    );
    Ok(())
}

#[test]
fn e2ee_notes_migration_locks_writers_before_preflight_and_trigger_installation() {
    let channels_lock = E2EE_NOTES_MIGRATION
        .find("LOCK TABLE channels IN SHARE ROW EXCLUSIVE MODE NOWAIT;")
        .expect("migration must acquire the channel lock without waiting");
    let notes_lock = E2EE_NOTES_MIGRATION
        .find("LOCK TABLE channel_notes IN SHARE ROW EXCLUSIVE MODE NOWAIT;")
        .expect("migration must acquire the note lock without waiting");
    let preflight = E2EE_NOTES_MIGRATION
        .find("USING ERRCODE = '55000';")
        .expect("migration must retain its data preflight");
    let trigger = E2EE_NOTES_MIGRATION
        .find("CREATE TRIGGER channel_notes_e2ee_reject_plaintext")
        .expect("migration must install the plaintext guard");

    assert!(channels_lock < notes_lock);
    assert!(notes_lock < preflight);
    assert!(preflight < trigger);
}

#[tokio::test]
async fn migration_write_barrier_racing_api_write_and_encryption_keeps_channel_safe()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let database_url = config.database_url.clone();
    let suffix = Uuid::new_v4().simple().to_string();
    let api_application_name = format!("notes_api_barrier_api_{suffix}");
    let conversion_application_name = format!("notes_api_barrier_conversion_{suffix}");
    config.database_url = database_url_with_application_name(&database_url, &api_application_name);
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let (status, body) = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"id":channel,"channel_name":format!("notes_migration_race_{}", Uuid::new_v4().simple()),"is_private":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (mut migration_connection, migration_driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(migration_driver);
    let blocker_pid: i32 = migration_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let migration_tx = migration_connection.transaction().await?;
    migration_tx
        .batch_execute(
            "LOCK TABLE channels IN SHARE ROW EXCLUSIVE MODE; LOCK TABLE channel_notes IN SHARE ROW EXCLUSIVE MODE;",
        )
        .await?;

    let direct_url =
        database_url_with_application_name(&database_url, &conversion_application_name);
    let direct_channel = channel.clone();
    let conversion = tokio::spawn(async move {
        let (connection, driver) =
            tokio_postgres::connect(&direct_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        connection
            .execute(
                "UPDATE channels SET encrypted = true WHERE id = $1",
                &[&direct_channel],
            )
            .await
    });
    let api = app.clone();
    let notes = format!("/api/channels/{channel}/notes");
    let actor = owner.clone();
    let write = tokio::spawn(async move {
        request(
            &api,
            "POST",
            &notes,
            &actor,
            json!({"title":"migration overlap","content":"plaintext"}),
        )
        .await
    });
    let (observer, observer_driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let conversion_pid =
        wait_until_application_blocked_by(&observer, &conversion_application_name, blocker_pid)
            .await?;
    let api_pid =
        wait_until_application_blocked_by(&observer, &api_application_name, blocker_pid).await?;
    assert_ne!(
        api_pid, conversion_pid,
        "API and conversion must use distinct sessions"
    );

    migration_tx.commit().await?;
    let (api_result, conversion_result) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(write, conversion)
        })
        .await?;
    let api_result = api_result?;
    let conversion_result = conversion_result?;
    match (api_result.0, conversion_result) {
        (StatusCode::OK, Err(error)) => assert_eq!(
            error.code().map(|code| code.code()),
            Some("23514"),
            "conversion must reject a note committed during the overlap"
        ),
        (StatusCode::BAD_REQUEST, Ok(_)) => {}
        (status, conversion) => panic!(
            "unsafe or unexpected migration race outcome: API {status}, conversion {conversion:?}, body {}",
            api_result.1
        ),
    }
    let state = observer
        .query_one(
            "SELECT c.encrypted, count(n.id) FROM channels c LEFT JOIN channel_notes n ON n.channel_id = c.id AND (n.deleted_at IS NULL OR n.deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours') WHERE c.id = $1 GROUP BY c.id",
            &[&channel],
        )
        .await?;
    let encrypted: bool = state.get(0);
    let plaintext_note_count: i64 = state.get(1);
    assert!(
        !encrypted || plaintext_note_count == 0,
        "encrypted channel must not retain active or restorable notes"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "mutates a table trigger; run alone against a disposable PostgreSQL database"]
async fn e2ee_migration_preflight_reports_sqlstate_55000_and_preserves_legacy_note()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let app = create_router(AppState::new(
        init_db(&config.database_url).await?,
        config.clone(),
    ));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let (status, body) = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"id":channel,"channel_name":format!("notes_migration_preflight_{}", Uuid::new_v4().simple()),"is_private":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let tx = connection.transaction().await?;
    tx.execute(
        "UPDATE channels SET encrypted = true WHERE id = $1",
        &[&channel],
    )
    .await?;
    tx.batch_execute("SAVEPOINT direct_note_attempt").await?;
    let rejected_note = tx
        .execute(
            "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ($1, $2, 'direct plaintext', 'body', $3)",
            &[&Uuid::new_v4().to_string(), &channel, &owner],
        )
        .await
        .expect_err("direct SQL must not insert plaintext into an encrypted channel");
    assert_eq!(rejected_note.code().map(|code| code.code()), Some("23514"));
    tx.batch_execute("ROLLBACK TO SAVEPOINT direct_note_attempt")
        .await?;
    tx.batch_execute(
        "ALTER TABLE channel_notes DISABLE TRIGGER channel_notes_e2ee_reject_plaintext",
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id) VALUES ($1, $2, 'legacy', 'plaintext', $3)",
        &[&Uuid::new_v4().to_string(), &channel, &owner],
    )
    .await?;

    let error = tx
        .batch_execute(E2EE_NOTES_MIGRATION)
        .await
        .expect_err("preflight must refuse to strand an encrypted plaintext note");
    assert_eq!(error.code().map(|code| code.code()), Some("55000"));
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn expired_notes_are_purged_without_touching_live_recent_or_locked_notes()
-> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("purge_{}", Uuid::new_v4().simple()),"is_private":false
    })).await;
    assert_eq!(created.0, StatusCode::OK);
    let collection = format!("/api/channels/{channel}/notes");
    let mut ids = Vec::new();
    for title in ["active", "recent", "expired", "locked"] {
        let (status, value) = request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":title,"content":"text"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        ids.push(value["data"]["id"].as_str().unwrap().to_owned());
    }
    for id in &ids[1..] {
        assert_eq!(
            request(
                &app,
                "DELETE",
                &format!("{collection}/{id}"),
                &owner,
                json!({"version":1})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection.execute("UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours' WHERE id = ANY($1)", &[&&ids[2..]]).await?;
    connection
        .execute(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            &[],
        )
        .await?;
    let locked = connection.transaction().await?;
    locked
        .query_one(
            "SELECT id FROM channel_notes WHERE id = $1 FOR UPDATE",
            &[&ids[3]],
        )
        .await?;
    let cleanup_db = db.clone();
    let mut cleanup = tokio::spawn(async move {
        backend::routes::channel_notes::purge_expired_notes(&cleanup_db).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut cleanup)
        .await
        .context("cleanup must skip row locks rather than wait")???;
    let rows = locked
        .query("SELECT id FROM channel_notes WHERE id = ANY($1)", &[&ids])
        .await?;
    let remaining: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    assert!(remaining.contains(&ids[0]));
    assert!(remaining.contains(&ids[1]));
    assert!(!remaining.contains(&ids[2]));
    assert!(remaining.contains(&ids[3]));
    locked.commit().await?;
    backend::routes::channel_notes::purge_expired_notes(&db).await?;
    assert!(
        connection
            .query_opt("SELECT id FROM channel_notes WHERE id=$1", &[&ids[3]])
            .await?
            .is_none()
    );
    backend::routes::channel_notes::purge_expired_notes(&db).await?;
    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_notes WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn expired_note_cleanup_skips_a_locked_channel_without_blocking_other_channels()
-> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let owner = Uuid::new_v4().to_string();

    // Cleanup visits channel IDs in sorted order. Holding the later channel
    // reproduces the old lock-and-wait behavior after it has locked the first.
    let channel_base = Uuid::new_v4().as_u128() & !0xff;
    let free_channel_a = Uuid::from_u128(channel_base + 1).to_string();
    let free_channel_b = Uuid::from_u128(channel_base + 2).to_string();
    let busy_channel = Uuid::from_u128(channel_base + 3).to_string();
    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    blocker_connection
        .execute(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, 'Cleanup test')",
            &[&owner],
        )
        .await?;
    for channel in [&free_channel_a, &free_channel_b, &busy_channel] {
        blocker_connection
            .execute(
                "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, false)",
                &[&channel, &format!("purge_{}", Uuid::new_v4().simple()), &owner],
            )
            .await?;
    }
    for channel in [&free_channel_a, &free_channel_b] {
        blocker_connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) \
                 SELECT gen_random_uuid()::text, $1, 'expired', 'text', $2, \
                        (statement_timestamp() AT TIME ZONE 'UTC') - interval '36500 days' \
                 FROM generate_series(1, 60)",
                &[&channel, &owner],
            )
            .await?;
    }
    blocker_connection
        .execute(
            "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) \
             VALUES (gen_random_uuid()::text, $1, 'expired', 'text', $2, \
                     (statement_timestamp() AT TIME ZONE 'UTC') - interval '36500 days')",
            &[&busy_channel, &owner],
        )
        .await?;
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&busy_channel],
        )
        .await?;

    // Reproduce the former multi-lock sequence on PostgreSQL: it holds the
    // first channel's advisory lock while waiting for the second one. A writer
    // for the first channel then cannot acquire its lock either.
    let (mut legacy_connection, legacy_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(legacy_driver);
    let legacy_free_channel = free_channel_a.clone();
    let legacy_busy_channel = busy_channel.clone();
    let legacy = tokio::spawn(async move {
        let tx = legacy_connection.transaction().await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&legacy_free_channel],
        )
        .await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&legacy_busy_channel],
        )
        .await?;
        tx.commit().await
    });
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    wait_until_blocked_by(&observer, blocker_pid)
        .await
        .context("legacy multi-channel lock acquisition waits on the held channel")?;
    let (mut writer_connection, writer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(writer_driver);
    let writer = writer_connection.transaction().await?;
    let first_channel_available: bool = writer
        .query_one(
            "SELECT pg_try_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&free_channel_a],
        )
        .await?
        .get(0);
    assert!(
        !first_channel_available,
        "the legacy cleanup waiter retains the earlier channel lock and blocks its writer"
    );
    writer.commit().await?;
    blocker.commit().await?;
    legacy.await??;

    blocker_connection
        .execute(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            &[],
        )
        .await?;
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
            &[&busy_channel],
        )
        .await?;

    let cleanup_db = db.clone();
    let cleanup = tokio::spawn(async move {
        backend::routes::channel_notes::purge_expired_notes(&cleanup_db).await
    });
    let mut cleanup = cleanup;
    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), &mut cleanup).await;
    let cleanup_result = match completed {
        Ok(result) => result??,
        Err(_) => {
            blocker.commit().await?;
            cleanup.await??;
            anyhow::bail!("cleanup waited on a channel lock while holding earlier channel locks")
        }
    };
    assert!(
        (1..=100).contains(&cleanup_result),
        "cleanup must make progress without exceeding its per-pass upper bound, got {cleanup_result}"
    );
    let remaining = blocker
        .query(
            "SELECT channel_id, count(*)::int FROM channel_notes WHERE channel_id IN ($1, $2, $3) GROUP BY channel_id",
            &[&free_channel_a, &free_channel_b, &busy_channel],
        )
        .await?;
    let remaining: std::collections::HashMap<String, i32> = remaining
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let remaining_a = remaining.get(&free_channel_a).copied().unwrap_or(0);
    let remaining_b = remaining.get(&free_channel_b).copied().unwrap_or(0);
    assert!(remaining_a < 60 && remaining_b < 60, "{remaining:?}");
    assert!(
        remaining_a.abs_diff(remaining_b) <= 1,
        "eligible channels should receive a fair share: {remaining:?}"
    );
    assert_eq!(remaining.get(&busy_channel), Some(&1), "{remaining:?}");
    blocker.commit().await?;

    for _ in 0..100 {
        let count = backend::routes::channel_notes::purge_expired_notes(&db).await?;
        assert!(count <= 100, "cleanup exceeded its per-pass upper bound");
        let remaining: i64 = blocker_connection
            .query_one(
                "SELECT count(*) FROM channel_notes WHERE channel_id IN ($1, $2, $3)",
                &[&free_channel_a, &free_channel_b, &busy_channel],
            )
            .await?
            .get(0);
        if remaining == 0 {
            break;
        }
        assert!(
            count > 0,
            "a skipped channel must be eligible on a later pass"
        );
    }
    let remaining: i64 = blocker_connection
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id IN ($1, $2, $3)",
            &[&free_channel_a, &free_channel_b, &busy_channel],
        )
        .await?
        .get(0);
    assert_eq!(
        remaining, 0,
        "all eligible and previously skipped notes are purged"
    );
    let cleanup = blocker_connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id IN ($1, $2, $3)",
            &[&free_channel_a, &free_channel_b, &busy_channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM channels WHERE id IN ($1, $2, $3)",
            &[&free_channel_a, &free_channel_b, &busy_channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly configured disposable NOTES_CLEANUP_TEST_DATABASE_URL"]
async fn expired_note_cleanup_reuses_unused_batch_capacity() -> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.database_url = std::env::var("NOTES_CLEANUP_TEST_DATABASE_URL")?;
    let db = init_db(&config.database_url).await?;
    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let database: String = connection
        .query_one("SELECT current_database()", &[])
        .await?
        .get(0);
    anyhow::ensure!(
        database.starts_with("zoo2_notes_cleanup_test_"),
        "refusing non-dedicated cleanup test database"
    );
    let owner = Uuid::new_v4().to_string();
    connection
        .execute(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1::text, $1::text, 'Cleanup batch test')",
            &[&owner],
        )
        .await?;
    let channels: Vec<String> = (0..5).map(|_| Uuid::new_v4().to_string()).collect();
    for (index, channel) in channels.iter().enumerate() {
        connection.execute("INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1::text, $1::text, $2, false)", &[channel, &owner]).await?;
        let amount: i32 = if index == 0 { 200 } else { 1 };
        connection.execute("INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) SELECT gen_random_uuid()::text, $1, 'expired', 'text', $2, (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours' FROM generate_series(1, $3::int)", &[channel, &owner, &amount]).await?;
    }
    let deleted = backend::routes::channel_notes::purge_expired_notes(&db).await?;
    assert_eq!(
        deleted, 100,
        "unused shares must be reassigned to channels with backlog"
    );
    let small_remaining: i64 = connection
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = ANY($1)",
            &[&&channels[1..]],
        )
        .await?
        .get(0);
    assert_eq!(
        small_remaining, 0,
        "each channel must receive a fair initial share"
    );
    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = ANY($1)",
            &[&channels],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = ANY($1)", &[&channels])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn expired_note_cleanup_scans_past_one_hundred_busy_channels() -> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let owner = Uuid::new_v4().to_string();
    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection
        .execute(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, 'Cleanup scan test')",
            &[&owner],
        )
        .await?;

    let channel_base = Uuid::new_v4().as_u128() & !0xff;
    let channels: Vec<String> = (1..=101)
        .map(|index| Uuid::from_u128(channel_base + index).to_string())
        .collect();
    for channel in &channels {
        connection
            .execute(
                "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, false)",
                &[channel, &format!("purge_{}", Uuid::new_v4().simple()), &owner],
            )
            .await?;
        connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) \
                 VALUES (gen_random_uuid()::text, $1, 'expired', 'text', $2, \
                         (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours')",
                &[channel, &owner],
            )
            .await?;
    }

    connection
        .execute(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            &[],
        )
        .await?;
    let blocker = connection.transaction().await?;
    for channel in &channels[..100] {
        blocker
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
                &[channel],
            )
            .await?;
    }

    let deleted = backend::routes::channel_notes::purge_expired_notes(&db).await?;
    assert!(
        (1..=100).contains(&deleted),
        "cleanup must reach later channels without exceeding its per-pass cap, got {deleted}"
    );
    let remaining: i64 = blocker
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = $1",
            &[&channels[100]],
        )
        .await?
        .get(0);
    assert_eq!(remaining, 0, "the later unlocked channel must be purged");

    blocker.commit().await?;
    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = ANY($1)",
            &[&channels],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = ANY($1)", &[&channels])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn expired_note_cleanup_rotates_across_a_persistent_backlog() -> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let owner = Uuid::new_v4().to_string();
    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection
        .execute(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, 'Cleanup fairness test')",
            &[&owner],
        )
        .await?;

    let channel_base = Uuid::new_v4().as_u128() & !0xff;
    let mut channels: Vec<String> = (1..=220)
        .map(|index| Uuid::from_u128(channel_base + index).to_string())
        .collect();
    for channel in &channels {
        connection
            .execute(
                "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, false)",
                &[channel, &format!("purge_fair_{}", Uuid::new_v4().simple()), &owner],
            )
            .await?;
        connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) \
                 SELECT gen_random_uuid()::text, $1, 'expired', 'text', $2, \
                        (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours' \
                 FROM generate_series(1, 20)",
                &[channel, &owner],
            )
            .await?;
    }

    let (first_worker, second_worker) =
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(
                backend::routes::channel_notes::purge_expired_notes(&db),
                backend::routes::channel_notes::purge_expired_notes(&db),
            )
        })
        .await?;
    let concurrent_deleted = first_worker? + second_worker?;
    assert!(
        concurrent_deleted <= 200,
        "concurrent cleanup workers must respect their per-pass caps"
    );

    // Grow the candidate set while cleanup is in progress. These one-note
    // channels also leave the eligible set as they are purged.
    let churn_channels: Vec<String> = (300..312)
        .map(|index| Uuid::from_u128(channel_base + index).to_string())
        .collect();
    for channel in &churn_channels {
        connection
            .execute(
                "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, false)",
                &[channel, &format!("purge_churn_{}", Uuid::new_v4().simple()), &owner],
            )
            .await?;
        connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, created_user_id, deleted_at) \
                 VALUES (gen_random_uuid()::text, $1, 'new expired', 'text', $2, \
                         (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours')",
                &[channel, &owner],
            )
            .await?;
    }
    channels.extend(churn_channels.iter().cloned());

    // With a growing and shrinking candidate set, repeated cleanup passes
    // must keep advancing instead of repeatedly choosing a fixed prefix.
    for _ in 0..48 {
        let deleted = backend::routes::channel_notes::purge_expired_notes(&db).await?;
        assert!(deleted <= 100, "cleanup exceeded its per-pass upper bound");
    }
    let counts = connection
        .query(
            "SELECT channel_id, count(*)::int FROM channel_notes WHERE channel_id = ANY($1) GROUP BY channel_id",
            &[&channels],
        )
        .await?;
    let remaining: std::collections::HashMap<String, i32> =
        counts.iter().map(|row| (row.get(0), row.get(1))).collect();
    let untouched: Vec<_> = channels
        .iter()
        .filter(|channel| remaining.get(*channel).copied().unwrap_or(0) == 20)
        .collect();
    assert!(
        untouched.is_empty(),
        "all channels in the persistent backlog must make progress; untouched={untouched:?}"
    );
    let remaining_churn: i64 = connection
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = ANY($1)",
            &[&churn_channels],
        )
        .await?
        .get(0);
    assert_eq!(remaining_churn, 0, "new candidates must not be starved");
    let remaining_notes: i64 = connection
        .query_one(
            "SELECT count(*) FROM channel_notes WHERE channel_id = ANY($1)",
            &[&channels],
        )
        .await?
        .get(0);
    assert_eq!(remaining_notes, 0, "all 220+ candidates must drain");

    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = ANY($1)",
            &[&channels],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = ANY($1)", &[&channels])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn deleted_note_timestamp_format_ignores_postgres_datestyle() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let separator = if config.database_url.contains('?') {
        '&'
    } else {
        '?'
    };
    config.database_url = format!(
        "{}{}options=-c%20DateStyle%3DSQL%2CDMY",
        config.database_url, separator
    );
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id":channel,
            "channel_name":format!("notes_datestyle_{}", Uuid::new_v4().simple()),
            "is_private":false
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let collection = format!("/api/channels/{channel}/notes");
    let (status, note) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"timestamp","content":"format"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    let note_id = note["data"]["id"].as_str().unwrap();

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let legacy_timestamp: String = connection
        .query_one(
            "SELECT replace(timestamp '2026-11-15 12:34:56.789'::text, ' ', 'T')",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(legacy_timestamp, "15/11/2026T12:34:56.789");
    let mut active_created_at = Value::Null;
    let mut active_updated_at = Value::Null;
    for (fraction, expected) in [("000", ""), ("120", ".12"), ("123", ".123")] {
        connection
            .execute(
                "UPDATE channel_notes SET created_at = $2::text::timestamp, updated_at = $3::text::timestamp WHERE id = $1",
                &[&note_id, &format!("2026-11-15 12:34:56.{fraction}"), &format!("2026-11-15 12:34:57.{fraction}")],
            )
            .await?;
        let (status, active) = request(&app, "GET", &collection, &owner, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{active}");
        let active_note = active["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == note_id)
            .context("created note missing from normal list")?;
        let created = format!("2026-11-15T12:34:56{expected}");
        let updated = format!("2026-11-15T12:34:57{expected}");
        assert_eq!(active_note["created_at"], created);
        assert_eq!(active_note["updated_at"], updated);
        active_created_at = active_note["created_at"].clone();
        active_updated_at = active_note["updated_at"].clone();
    }

    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{note_id}"),
            &owner,
            json!({"version":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    connection
        .execute(
            "UPDATE channel_notes SET updated_at = timestamp '2026-11-15 12:34:57.123' WHERE id = $1",
            &[&note_id],
        )
        .await?;
    let (status, deleted) = request(
        &app,
        "GET",
        &format!("{collection}/deleted"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    let deleted_note = deleted["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == note_id)
        .context("deleted note missing from deleted list")?;
    assert_eq!(deleted_note["created_at"], active_created_at);
    assert_eq!(deleted_note["updated_at"], active_updated_at);
    assert_eq!(deleted_note["created_at"], "2026-11-15T12:34:56.123");
    assert_eq!(deleted_note["updated_at"], "2026-11-15T12:34:57.123");

    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn channel_notes_wait_for_channel_and_membership_changes_before_authorizing()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &member).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner,
        json!({"id":channel,"channel_name":format!("lock_{}", Uuid::new_v4().simple()),"is_private":false})).await.0, StatusCode::OK);
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/users"),
            &owner,
            json!({"user_id":member})
        )
        .await
        .0,
        StatusCode::OK
    );
    let collection = format!("/api/channels/{channel}/notes");

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);

    // An uncommitted archive holds the row lock while authorization sees the old state.
    let tx = connection.transaction().await?;
    tx.execute(
        "UPDATE channels SET archive_at = now() WHERE id = $1",
        &[&channel],
    )
    .await?;
    let api = app.clone();
    let path = collection.clone();
    let actor = owner.clone();
    let pending = tokio::spawn(async move {
        request(
            &api,
            "POST",
            &path,
            &actor,
            json!({"title":"Race","content":""}),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !pending.is_finished(),
        "write must wait for the archive transaction"
    );
    tx.commit().await?;
    assert_eq!(pending.await?.0, StatusCode::BAD_REQUEST);

    connection
        .execute(
            "UPDATE channels SET archive_at = NULL WHERE id = $1",
            &[&channel],
        )
        .await?;
    // Membership removal similarly holds the row until commit.
    let tx = connection.transaction().await?;
    tx.execute(
        "DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2",
        &[&channel, &member],
    )
    .await?;
    let api = app.clone();
    let path = collection.clone();
    let actor = member.clone();
    let pending = tokio::spawn(async move {
        request(
            &api,
            "POST",
            &path,
            &actor,
            json!({"title":"Race","content":""}),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !pending.is_finished(),
        "write must wait for membership removal"
    );
    tx.commit().await?;
    assert_eq!(pending.await?.0, StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn channel_notes_get_waits_for_visibility_and_membership_changes() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    let outsider = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &member).await;
    create_user(&app, &outsider).await;

    let public_channel = Uuid::new_v4().to_string();
    let public_name = format!("notes_get_{}", Uuid::new_v4().simple());
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({
                "id":public_channel,"channel_name":public_name,"is_private":false
            })
        )
        .await
        .0,
        StatusCode::OK
    );
    let collection = format!("/api/channels/{public_channel}/notes");
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":"secret after privatization","content":"body"})
        )
        .await
        .0,
        StatusCode::OK
    );

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let tx = connection.transaction().await?;
    tx.execute(
        "UPDATE channels SET is_private = true WHERE id = $1",
        &[&public_channel],
    )
    .await?;
    let api = app.clone();
    let path = collection.clone();
    let actor = outsider.clone();
    let pending =
        tokio::spawn(async move { request(&api, "GET", &path, &actor, Value::Null).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !pending.is_finished(),
        "GET must wait for a concurrent visibility change"
    );
    tx.commit().await?;
    assert_eq!(pending.await?.0, StatusCode::NOT_FOUND);

    let private_channel = Uuid::new_v4().to_string();
    let private_name = format!("notes_get_{}", Uuid::new_v4().simple());
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({
                "id":private_channel,"channel_name":private_name,"is_private":true
            })
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{private_channel}/users"),
            &owner,
            json!({"user_id":member})
        )
        .await
        .0,
        StatusCode::OK
    );
    let private_notes = format!("/api/channels/{private_channel}/notes");
    assert_eq!(
        request(
            &app,
            "POST",
            &private_notes,
            &owner,
            json!({"title":"member secret","content":"body"})
        )
        .await
        .0,
        StatusCode::OK
    );

    let tx = connection.transaction().await?;
    // Model app/webhook/Slack membership cleanup, which deletes channel_user
    // without first locking the channel row.
    tx.execute(
        "DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2",
        &[&private_channel, &member],
    )
    .await?;
    let api = app.clone();
    let path = private_notes;
    let actor = member;
    let pending =
        tokio::spawn(async move { request(&api, "GET", &path, &actor, Value::Null).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !pending.is_finished(),
        "GET must wait for membership removal even when it does not lock the channel"
    );
    tx.commit().await?;
    assert_eq!(pending.await?.0, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn adding_user_serializes_guest_transition_before_channel_lock() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let guest = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &guest).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_guest_lock_{}", Uuid::new_v4().simple()),"is_private":false
    })).await.0, StatusCode::OK);

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT id FROM channels WHERE id = $1 FOR UPDATE",
            &[&channel],
        )
        .await?;

    let add_app = app.clone();
    let add_path = format!("/api/channels/{channel}/users");
    let add_guest = guest.clone();
    let add = tokio::spawn(async move {
        request(
            &add_app,
            "POST",
            &add_path,
            &owner,
            json!({"user_id":add_guest}),
        )
        .await
    });
    let (observe_connection, observe_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observe_driver);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = observe_connection.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%channels%' AND query LIKE '%NO KEY UPDATE%' AND $1 = ANY(pg_blocking_pids(pid)))",
                &[&blocker_pid],
            ).await?.get(0);
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok::<(), anyhow::Error>(())
    }).await??;

    let (update_connection, update_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(update_driver);
    let update_guest = guest.clone();
    let update = tokio::spawn(async move {
        update_connection
            .execute(
                "UPDATE users SET guest_kind = 'single' WHERE id = $1",
                &[&update_guest],
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let transition_waited_for_add = !update.is_finished();
    assert!(
        transition_waited_for_add,
        "a guest_kind transition must wait while add_channel_user holds the user row"
    );

    // Release the channel, allow the add to finish, then let the transition
    // commit. With the former conditional lock, the transition completed while
    // add waited on the channel; a notes GET then reproduced the deadlock.
    blocker.commit().await?;
    let add_response = tokio::time::timeout(std::time::Duration::from_secs(10), add).await??;
    assert_eq!(add_response.0, StatusCode::OK, "{}", add_response.1);
    update.await??;

    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/api/channels/{channel}/notes"),
            &guest,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn channel_creation_rechecks_guest_kind_after_waiting_for_user_lock() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let user = Uuid::new_v4().to_string();
    create_user(&app, &user).await;

    let (mut transition_connection, transition_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(transition_driver);
    let transition_pid: i32 = transition_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let transition = transition_connection.transaction().await?;
    transition
        .execute(
            "UPDATE users SET guest_kind = 'multi' WHERE id = $1",
            &[&user],
        )
        .await?;

    let create_app = app.clone();
    let create_user_id = user.clone();
    let create = tokio::spawn(async move {
        request(
            &create_app,
            "POST",
            "/api/channels",
            &create_user_id,
            json!({
                "channel_name": format!("guest_create_race_{}", Uuid::new_v4().simple()),
                "is_private": false
            }),
        )
        .await
    });
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = observer
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%FOR NO KEY UPDATE%' AND $1 = ANY(pg_blocking_pids(pid)))",
                    &[&transition_pid],
                )
                .await?
                .get(0);
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok::<(), tokio_postgres::Error>(())
    })
    .await??;

    transition.commit().await?;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), create).await??;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    Ok(())
}

#[tokio::test]
async fn channel_note_deleted_at_is_utc_in_non_utc_sessions() -> anyhow::Result<()> {
    let _cleanup_guard = CLEANUP_TEST_LOCK.lock().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let separator = if config.database_url.contains('?') {
        '&'
    } else {
        '?'
    };
    config.database_url = format!(
        "{}{}options=-c%20timezone%3DAsia/Tokyo",
        config.database_url, separator
    );
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_tz_{}", Uuid::new_v4().simple()),"is_private":false
    })).await.0, StatusCode::OK);
    let collection = format!("/api/channels/{channel}/notes");
    let (status, created) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"timezone","content":"body"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let note_id = created["data"]["id"].as_str().unwrap();
    let first_delete_started = tokio::time::Instant::now();
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{note_id}"),
            &owner,
            json!({"version":1})
        )
        .await
        .0,
        StatusCode::OK
    );

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let rows = connection.query(
        "SELECT current_setting('TimeZone'), EXTRACT(EPOCH FROM (statement_timestamp() - (deleted_at AT TIME ZONE 'UTC')))::double precision, EXTRACT(EPOCH FROM (statement_timestamp() - (updated_at AT TIME ZONE 'UTC')))::double precision FROM channel_notes WHERE id = $1",
        &[&note_id],
    ).await?;
    assert_eq!(rows[0].get::<_, String>(0), "Asia/Tokyo");
    let age_seconds: f64 = rows[0].get(1);
    let max_age_seconds = first_delete_started.elapsed().as_secs_f64() + 0.1;
    assert!(
        (0.0..=max_age_seconds).contains(&age_seconds),
        "deleted_at must be stored as UTC, age={age_seconds}"
    );
    let updated_age_seconds: f64 = rows[0].get(2);
    assert!(
        (0.0..=max_age_seconds).contains(&updated_age_seconds),
        "remove updated_at must be stored as UTC, age={updated_age_seconds}"
    );
    let restore_started = tokio::time::Instant::now();
    let (status, restored) = request(
        &app,
        "POST",
        &format!("{collection}/{note_id}/restore"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    let rows = connection.query(
        "SELECT EXTRACT(EPOCH FROM (statement_timestamp() - (updated_at AT TIME ZONE 'UTC')))::double precision FROM channel_notes WHERE id = $1",
        &[&note_id],
    ).await?;
    let age_seconds: f64 = rows[0].get(0);
    assert!(
        (0.0..=restore_started.elapsed().as_secs_f64() + 0.1).contains(&age_seconds),
        "restore updated_at must be stored as UTC, age={age_seconds}"
    );
    let update_started = tokio::time::Instant::now();
    let (status, updated) = request(
        &app,
        "PATCH",
        &format!("{collection}/{note_id}"),
        &owner,
        json!({"title":"changed","content":"body","version":3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let rows = connection.query(
        "SELECT EXTRACT(EPOCH FROM (statement_timestamp() - (updated_at AT TIME ZONE 'UTC')))::double precision FROM channel_notes WHERE id = $1",
        &[&note_id],
    ).await?;
    let age_seconds: f64 = rows[0].get(0);
    assert!(
        (0.0..=update_started.elapsed().as_secs_f64() + 0.1).contains(&age_seconds),
        "update updated_at must be stored as UTC, age={age_seconds}"
    );

    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{note_id}"),
            &owner,
            json!({"version":4})
        )
        .await
        .0,
        StatusCode::OK
    );
    connection.execute(
        "UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '23 hours 59 minutes' WHERE id = $1",
        &[&note_id],
    ).await?;
    let (status, listed) = request(
        &app,
        "GET",
        &format!("{collection}/deleted"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["id"] == note_id),
        "a note still inside the 24-hour window must appear in the deleted list"
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{collection}/{note_id}/restore"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK,
        "a note inside the 24-hour window must restore"
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{note_id}"),
            &owner,
            json!({"version":6})
        )
        .await
        .0,
        StatusCode::OK
    );
    connection.execute(
        "UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours 1 second' WHERE id = $1",
        &[&note_id],
    ).await?;
    let (status, listed) = request(
        &app,
        "GET",
        &format!("{collection}/deleted"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        !listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["id"] == note_id),
        "a note outside the 24-hour window must not appear in the deleted list"
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{collection}/{note_id}/restore"),
            &owner,
            Value::Null,
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "an expired tombstone must not restore before cleanup"
    );
    connection
        .execute(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            &[],
        )
        .await?;
    backend::routes::channel_notes::purge_expired_notes(&db).await?;
    assert!(
        connection
            .query_opt("SELECT id FROM channel_notes WHERE id = $1", &[&note_id])
            .await?
            .is_none(),
        "expired tombstones must be physically purged"
    );
    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_notes WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn deleted_note_window_uses_database_clock_when_application_clock_disagrees()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let (setup, setup_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(setup_driver);
    setup
        .batch_execute(
            "CREATE SCHEMA IF NOT EXISTS notes_clock_test; \
             CREATE OR REPLACE FUNCTION notes_clock_test.statement_timestamp() RETURNS timestamptz \
             LANGUAGE sql STABLE AS $$ SELECT pg_catalog.statement_timestamp() + interval '26 hours' $$;",
        )
        .await?;

    let separator = if config.database_url.contains('?') {
        '&'
    } else {
        '?'
    };
    config.database_url = format!(
        "{}{}options=-c%20search_path%3Dnotes_clock_test%2Cpublic%2Cpg_catalog",
        config.database_url, separator
    );
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_clock_{}", Uuid::new_v4().simple()),"is_private":false
    })).await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);
    let collection = format!("/api/channels/{channel}/notes");
    let (status, note) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"clock-skew","content":"body"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    let note_id = note["data"]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{note_id}"),
            &owner,
            json!({"version":1}),
        )
        .await
        .0,
        StatusCode::OK
    );

    let (connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let skew_hours: f64 = connection
        .query_one(
            "SELECT (EXTRACT(EPOCH FROM (statement_timestamp() - pg_catalog.statement_timestamp())) / 3600.0)::double precision",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(skew_hours, 26.0, "the test DB clock is 26 hours ahead");
    connection
        .execute(
            "UPDATE channel_notes SET deleted_at = \
             (pg_catalog.statement_timestamp() + interval '1 hour') AT TIME ZONE 'UTC' \
             WHERE id = $1",
            &[&note_id],
        )
        .await?;

    let (status, listed) = request(
        &app,
        "GET",
        &format!("{collection}/deleted"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        !listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == note_id),
        "DB time sees the tombstone as 25 hours old even though app time sees it one hour in the future"
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{collection}/{note_id}/restore"),
            &owner,
            Value::Null,
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "restore must use the same database cutoff as listing"
    );
    Ok(())
}

#[tokio::test]
async fn channel_note_create_and_update_reject_nul_in_title_or_content() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_nul_{}", Uuid::new_v4().simple()),"is_private":false
    })).await.0, StatusCode::OK);
    let collection = format!("/api/channels/{channel}/notes");
    for field in ["title", "content"] {
        let mut input = json!({"title":"valid title","content":"valid content"});
        input[field] = Value::String("invalid\0text".into());
        assert_eq!(
            request(&app, "POST", &collection, &owner, input).await.0,
            StatusCode::BAD_REQUEST,
            "create accepted NUL in {field}"
        );
    }
    let (status, created) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"valid title","content":"valid content"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let note_id = created["data"]["id"].as_str().unwrap();
    for field in ["title", "content"] {
        let mut input = json!({"title":"valid title","content":"valid content","version":1});
        input[field] = Value::String("invalid\0text".into());
        assert_eq!(
            request(
                &app,
                "PATCH",
                &format!("{collection}/{note_id}"),
                &owner,
                input
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "update accepted NUL in {field}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn channel_notes_get_rechecks_deactivation_after_authentication() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let actor = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &actor).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_deactivate_{}", Uuid::new_v4().simple()),"is_private":false
    })).await.0, StatusCode::OK);
    let collection = format!("/api/channels/{channel}/notes");
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":"must stay private after deactivation","content":"body"})
        )
        .await
        .0,
        StatusCode::OK
    );

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .batch_execute("LOCK TABLE channel_notes IN ACCESS EXCLUSIVE MODE")
        .await?;
    let (mut update_connection, update_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(update_driver);
    let update = update_connection.transaction().await?;
    update
        .execute(
            "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
            &[&actor],
        )
        .await?;

    let api = app.clone();
    let path = collection;
    let request_actor = actor.clone();
    let pending =
        tokio::spawn(async move { request(&api, "GET", &path, &request_actor, Value::Null).await });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(
        !pending.is_finished(),
        "GET should reach the blocked note query after AuthUser's separate active check"
    );
    update.commit().await?;
    blocker.commit().await?;
    assert_eq!(
        pending.await?.0,
        StatusCode::FORBIDDEN,
        "a request whose AuthUser check preceded deactivation must recheck active state in its transaction"
    );
    Ok(())
}

#[tokio::test]
async fn channel_notes_get_rechecks_guest_status_before_public_read() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let actor = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &actor).await;
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({
        "id":channel,"channel_name":format!("notes_guest_{}", Uuid::new_v4().simple()),"is_private":false
    })).await.0, StatusCode::OK);
    let collection = format!("/api/channels/{channel}/notes");
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":"non-member public note","content":"body"})
        )
        .await
        .0,
        StatusCode::OK
    );

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .batch_execute("LOCK TABLE channel_notes IN ACCESS EXCLUSIVE MODE")
        .await?;
    let (mut update_connection, update_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(update_driver);
    let update = update_connection.transaction().await?;
    update
        .execute(
            "UPDATE users SET guest_kind = 'multi' WHERE id = $1",
            &[&actor],
        )
        .await?;

    let api = app.clone();
    let path = collection;
    let request_actor = actor.clone();
    let pending =
        tokio::spawn(async move { request(&api, "GET", &path, &request_actor, Value::Null).await });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(
        !pending.is_finished(),
        "GET should reach the blocked note query after guest authorization"
    );
    update.commit().await?;
    blocker.commit().await?;
    assert_eq!(
        pending.await?.0,
        StatusCode::NOT_FOUND,
        "a user changed to guest after authorization must not read public notes without membership"
    );
    Ok(())
}

#[tokio::test]
async fn channel_notes_enforce_access_and_compare_versions_atomically() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    let outsider = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    create_user(&app, &member).await;
    create_user(&app, &outsider).await;

    let channel = Uuid::new_v4().to_string();
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/channels",
            &owner,
            json!({"id":channel,"channel_name":format!("notes_{}", Uuid::new_v4().simple()),"is_private":false}),
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/users"),
            &owner,
            json!({"user_id":member}),
        )
        .await
        .0,
        StatusCode::OK
    );
    let collection = format!("/api/channels/{channel}/notes");

    // Public-channel read follows message visibility; writes require membership.
    let (list_status, list_body) = request(&app, "GET", &collection, &outsider, Value::Null).await;
    assert_eq!(list_status, StatusCode::OK, "{list_body}");
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &outsider,
            json!({"title":"No","content":"write"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, created) = request(
        &app,
        "POST",
        &collection,
        &member,
        json!({"title":"Runbook","content":"First draft"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!created["txid"].as_str().unwrap_or_default().is_empty());
    let note = &created["data"];
    let note_id = note["id"].as_str().unwrap();
    assert_eq!(note["version"], 1);

    // A member can read and update the note; stale update and stale delete both conflict.
    let item = format!("{collection}/{note_id}");
    assert_eq!(
        request(&app, "GET", &collection, &outsider, Value::Null)
            .await
            .1["data"][0]["title"],
        "Runbook"
    );
    let (first, second) = tokio::join!(
        request(
            &app,
            "PATCH",
            &item,
            &owner,
            json!({"title":"Runbook v2a","content":"Concurrent draft A","version":1}),
        ),
        request(
            &app,
            "PATCH",
            &item,
            &member,
            json!({"title":"Runbook v2b","content":"Concurrent draft B","version":1}),
        ),
    );
    assert_eq!(
        [first.0, second.0]
            .into_iter()
            .filter(|status| *status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [first.0, second.0]
            .into_iter()
            .filter(|status| *status == StatusCode::CONFLICT)
            .count(),
        1
    );
    let updated = if first.0 == StatusCode::OK {
        first.1
    } else {
        second.1
    };
    assert_eq!(updated["data"]["version"], 2);
    assert_eq!(
        request(
            &app,
            "PATCH",
            &item,
            &member,
            json!({"title":"Stale","content":"Lost","version":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "DELETE", &item, &member, json!({"version":1}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let current_version = 2;
    let created_second = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"Second","content":""}),
    )
    .await
    .1;
    let second_id = created_second["data"]["id"].as_str().unwrap();
    // Deletion is recoverable, omitted from the active list, and restores for members only.
    assert_eq!(
        request(
            &app,
            "DELETE",
            &item,
            &member,
            json!({"version":current_version})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (timestamp_connection, timestamp_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(timestamp_driver);
    let deleted_updated_at: String = timestamp_connection
        .query_one(
            "SELECT replace(updated_at::text, ' ', 'T') FROM channel_notes WHERE id = $1",
            &[&note_id],
        )
        .await?
        .get(0);
    assert_eq!(
        request(&app, "GET", &collection, &member, Value::Null)
            .await
            .1["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let deleted = format!("{collection}/deleted");
    assert_eq!(
        request(&app, "GET", &deleted, &outsider, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let deleted_list = request(&app, "GET", &deleted, &member, Value::Null).await.1;
    assert_eq!(deleted_list["data"][0]["id"], note_id);
    assert_eq!(deleted_list["data"][0]["title"], updated["data"]["title"]);
    assert_eq!(
        deleted_list["data"][0]["content"],
        updated["data"]["content"]
    );
    assert_eq!(
        deleted_list["data"][0]["created_at"],
        created["data"]["created_at"]
    );
    assert_eq!(deleted_list["data"][0]["updated_at"], deleted_updated_at);
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{item}/restore"),
            &outsider,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{item}/restore"),
            &member,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", &collection, &member, Value::Null)
            .await
            .1["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        request(&app, "GET", &collection, &member, Value::Null)
            .await
            .1["data"][0]["id"],
        note_id,
        "restored notes retain their original position when possible"
    );

    // Order is shared state and must include all active notes exactly once.
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("{collection}/order"),
            &member,
            json!({"expected_note_ids":[note_id,second_id],"note_ids":[second_id,note_id]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("{collection}/order"),
            &owner,
            json!({"expected_note_ids":[note_id,second_id],"note_ids":[note_id,second_id]})
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "a reorder based on an older order must not overwrite another client's update"
    );
    assert_eq!(
        request(&app, "GET", &collection, &outsider, Value::Null)
            .await
            .1["data"][0]["id"],
        second_id
    );
    let expired = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"Expired","content":""}),
    )
    .await
    .1;
    let expired_id = expired["data"]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{expired_id}"),
            &owner,
            json!({"version":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut expiry_db = db.clone();
    let mut expiry_tx = expiry_db.transaction().await?;
    toasty::sql::query(
        "UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours' WHERE id = $1",
    )
    .bind(expired_id)
    .exec(&mut expiry_tx)
    .await?;
    expiry_tx.commit().await?;
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{collection}/{expired_id}/restore"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // Private-channel visibility is hidden from non-members even for direct note URLs.
    let private_channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({"id":private_channel,"channel_name":format!("private_notes_{}", Uuid::new_v4().simple()),"is_private":true})).await.0, StatusCode::OK);
    let private_notes = format!("/api/channels/{private_channel}/notes");
    assert_eq!(
        request(&app, "GET", &private_notes, &outsider, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &private_notes,
            &outsider,
            json!({"title":"Secret","content":"No"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // Archived channels remain readable, but note mutations stop.
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/archive"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", &collection, &member, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            &item,
            &member,
            json!({"title":"No edit","content":"No","version":2})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "DELETE", &item, &member, json!({"version":2}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // DM notes follow DM membership even if a guessed channel ID is known.
    let (status, dm) = request(
        &app,
        "POST",
        "/api/dms",
        &owner,
        json!({"target_user_ids":[member]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let dm_channel = dm["data"]["id"].as_str().unwrap();
    let dm_notes = format!("/api/channels/{dm_channel}/notes");
    assert_eq!(
        request(&app, "GET", &dm_notes, &outsider, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, dm_note) = request(
        &app,
        "POST",
        &dm_notes,
        &owner,
        json!({"title":"Private","content":"DM only"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request(&app, "GET", &dm_notes, &member, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("{dm_notes}/{}", dm_note["data"]["id"].as_str().unwrap()),
            &outsider,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("/api/channels/{channel}"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", &collection, &member, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn channel_note_update_ignores_soft_deleted_note_with_post_delete_version()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel,
            "channel_name": format!("notes_deleted_patch_{}", Uuid::new_v4().simple()),
            "is_private": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let collection = format!("/api/channels/{channel}/notes");
    let (status, note) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"Original","content":"Before delete"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    let note_id = note["data"]["id"].as_str().unwrap();
    let item = format!("{collection}/{note_id}");

    assert_eq!(
        request(&app, "DELETE", &item, &owner, json!({"version":1}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            &item,
            &owner,
            json!({"title":"Mutated","content":"While deleted","version":2})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let (status, restored) = request(
        &app,
        "POST",
        &format!("{item}/restore"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["data"]["title"], "Original");
    assert_eq!(restored["data"]["content"], "Before delete");
    assert_eq!(restored["data"]["version"], 3);

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn channel_note_restore_expired_does_not_shift_active_positions() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel,
            "channel_name": format!("notes_expired_pos_{}", Uuid::new_v4().simple()),
            "is_private": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let collection = format!("/api/channels/{channel}/notes");
    let (status, first) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"First","content":"1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_id = first["data"]["id"].as_str().unwrap();

    let (status, second) = request(
        &app,
        "POST",
        &collection,
        &owner,
        json!({"title":"Second","content":"2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_id = second["data"]["id"].as_str().unwrap();

    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("{collection}/{first_id}"),
            &owner,
            json!({"version":1})
        )
        .await
        .0,
        StatusCode::OK
    );

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection
        .execute(
            "UPDATE channel_notes SET deleted_at = (statement_timestamp() AT TIME ZONE 'UTC') - interval '25 hours' WHERE id = $1",
            &[&first_id],
        )
        .await?;

    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{collection}/{first_id}/restore"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let second_pos: i32 = connection
        .query_one(
            "SELECT position FROM channel_notes WHERE id = $1",
            &[&second_id],
        )
        .await?
        .get(0);
    assert_eq!(second_pos, 1);

    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}

#[tokio::test]
async fn channel_note_duplicate_positions_tie_break_deterministically() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    create_user(&app, &owner).await;
    let channel = Uuid::new_v4().to_string();
    let created = request(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({
            "id": channel,
            "channel_name": format!("notes_dup_pos_{}", Uuid::new_v4().simple()),
            "is_private": true
        }),
    )
    .await;
    assert_eq!(created.0, StatusCode::OK, "{}", created.1);

    let mut id_a = Uuid::new_v4().to_string();
    let mut id_b = Uuid::new_v4().to_string();
    if id_a > id_b {
        std::mem::swap(&mut id_a, &mut id_b);
    }
    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    // Insert the higher ID first at position 0, then the lower ID at the same
    // position so heap scan order differs from (position, id) order.
    for (id, title) in [(&id_b, "High ID"), (&id_a, "Low ID")] {
        connection
            .execute(
                "INSERT INTO channel_notes (id, channel_id, title, content, position, version, created_user_id, created_at, updated_at)
                 VALUES ($1, $2, $3, 'body', 0, 1, $4, statement_timestamp() AT TIME ZONE 'UTC', statement_timestamp() AT TIME ZONE 'UTC')",
                &[id, &channel, &title, &owner],
            )
            .await?;
    }

    let collection = format!("/api/channels/{channel}/notes");
    let (status, listed) = request(&app, "GET", &collection, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let listed_ids: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(listed_ids, vec![id_a.as_str(), id_b.as_str()]);

    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("{collection}/order"),
            &owner,
            json!({"expected_note_ids":[id_b, id_a],"note_ids":[id_b, id_a]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("{collection}/order"),
            &owner,
            json!({"expected_note_ids":[id_a, id_b],"note_ids":[id_b, id_a]})
        )
        .await
        .0,
        StatusCode::OK
    );

    let cleanup = connection.transaction().await?;
    cleanup
        .execute("SET LOCAL zoo.suppress_app_events = 'on'", &[])
        .await?;
    cleanup
        .execute(
            "DELETE FROM channel_user WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute(
            "DELETE FROM app_deliveries WHERE channel_id = $1",
            &[&channel],
        )
        .await?;
    cleanup
        .execute("DELETE FROM channels WHERE id = $1", &[&channel])
        .await?;
    cleanup
        .execute("DELETE FROM users WHERE id = $1", &[&owner])
        .await?;
    cleanup.commit().await?;
    Ok(())
}
