use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

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

#[tokio::test]
async fn notes_hide_public_channels_from_nonmember_guests() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
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
async fn expired_notes_are_purged_without_touching_live_recent_or_locked_notes()
-> anyhow::Result<()> {
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
    connection.execute("UPDATE channel_notes SET deleted_at = statement_timestamp() - interval '25 hours' WHERE id = ANY($1)", &[&&ids[2..]]).await?;
    let locked = connection.transaction().await?;
    locked
        .query_one(
            "SELECT id FROM channel_notes WHERE id = $1 FOR UPDATE",
            &[&ids[3]],
        )
        .await?;
    backend::routes::channel_notes::purge_expired_notes(&db).await?;
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
async fn channel_notes_enforce_access_and_compare_versions_atomically() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
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
    assert_eq!(
        request(&app, "GET", &deleted, &member, Value::Null).await.1["data"][0]["id"],
        note_id
    );
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
        "UPDATE channel_notes SET deleted_at = statement_timestamp() - interval '25 hours' WHERE id = $1",
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
