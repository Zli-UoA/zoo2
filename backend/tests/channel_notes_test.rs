use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use backend::{
    config::Config, db::init_db, models::now_utc_millis, routes::create_router, state::AppState,
};

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

async fn note_notification_count(
    db: &mut toasty::Db,
    note_id: &str,
    user_id: &str,
) -> anyhow::Result<i64> {
    let rows = toasty::sql::query(
        "SELECT count(*) AS n FROM notification_events WHERE note_id = $1 AND user_id = $2",
    )
    .bind(note_id)
    .bind(user_id)
    .exec(db)
    .await?;
    match rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::I64(value)) => Ok(*value),
            _ => anyhow::bail!("notification count was not an i64"),
        },
        _ => anyhow::bail!("notification count query returned no row"),
    }
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
async fn notes_follow_guest_visibility_and_channel_posting_policy() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    let guest = Uuid::new_v4().to_string();
    for user in [&owner, &member, &guest] {
        create_user(&app, user).await;
    }
    let channel = Uuid::new_v4().to_string();
    assert_eq!(request(&app, "POST", "/api/channels", &owner,
        json!({"id":channel,"channel_name":format!("policy_{}",Uuid::new_v4().simple()),"is_private":false})).await.0, StatusCode::OK);
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
    let (connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    connection
        .execute("UPDATE users SET guest_kind='multi' WHERE id=$1", &[&guest])
        .await?;
    connection
        .execute(
            "UPDATE channels SET posting_policy='managers' WHERE id=$1",
            &[&channel],
        )
        .await?;
    let collection = format!("/api/channels/{channel}/notes");
    assert_eq!(
        request(&app, "GET", &collection, &guest, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &member,
            json!({"title":"Denied","content":""})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &collection,
            &owner,
            json!({"title":"Allowed","content":""})
        )
        .await
        .0,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn channel_notes_enforce_access_and_compare_versions_atomically() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    let outsider = Uuid::new_v4().to_string();
    let owner_name = create_user(&app, &owner).await;
    let member_name = create_user(&app, &member).await;
    let outsider_name = create_user(&app, &outsider).await;

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
    let (status, summary) = request(
        &app,
        "GET",
        &format!("{collection}?summary=true"),
        &outsider,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        summary["data"][0].get("content").is_none(),
        "summary must not include note bodies"
    );

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
    let mention_content = format!("hello @{member_name} and @{owner_name}");
    let (status, first_mention) = request(
        &app,
        "PATCH",
        &item,
        &owner,
        json!({"title":"Runbook","content":mention_content,"version":2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first_mention}");
    assert_eq!(note_notification_count(&mut db, note_id, &member).await?, 1);
    assert_eq!(note_notification_count(&mut db, note_id, &owner).await?, 0);
    let mut tx = db.transaction().await?;
    let legacy = backend::apps::one(&mut tx,
        "SELECT json_build_object('count',count(*))::text FROM notifications WHERE id IN (SELECT id FROM notification_events WHERE note_id=$1)",
        &[note_id]).await?;
    assert_eq!(
        legacy["count"], 0,
        "note notifications must not be written to the legacy message-only table"
    );
    drop(tx);
    request(
        &app,
        "PATCH",
        &item,
        &owner,
        json!({"title":"Runbook","content":mention_content,"version":3}),
    )
    .await;
    assert_eq!(
        note_notification_count(&mut db, note_id, &member).await?,
        1,
        "unchanged target is not re-notified"
    );
    request(
        &app,
        "PATCH",
        &item,
        &owner,
        json!({"title":"Runbook","content":"mention removed","version":4}),
    )
    .await;
    request(
        &app,
        "PATCH",
        &item,
        &owner,
        json!({"title":"Runbook","content":mention_content,"version":5}),
    )
    .await;
    assert_eq!(
        note_notification_count(&mut db, note_id, &member).await?,
        2,
        "a removed and re-added target is notified again"
    );
    let current_version = 6;
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

    // Note mentions notify only newly targeted members, are addressable as note context,
    // and disappear from that context while the note is deleted.
    let notification_note = request(
        &app,
        "POST",
        &collection,
        &member,
        json!({"title":"Mentions","content":""}),
    )
    .await
    .1["data"]
        .clone();
    let notification_note_id = notification_note["id"].as_str().unwrap();
    let notification_note_path = format!("{collection}/{notification_note_id}");
    let bot = Uuid::new_v4().to_string();
    create_user(&app, &bot).await;
    toasty::sql::query("UPDATE users SET is_bot=true WHERE id=$1")
        .bind(bot.as_str())
        .exec(&mut db)
        .await?;
    backend::models::ChannelUser::create()
        .user_id(bot.as_str())
        .channel_id(channel.as_str())
        .joined_at(now_utc_millis())
        .exec(&mut db)
        .await?;
    backend::models::ChannelUser::create()
        .user_id(outsider.as_str())
        .channel_id(channel.as_str())
        .joined_at(now_utc_millis())
        .exec(&mut db)
        .await?;
    let first = request(&app, "PATCH", &notification_note_path, &owner,
        json!({"title":"Mentions","content":format!("Hi @{member_name} `@{owner_name}` [ref](https://example.test/@outsider_name)"),"version":1})).await;
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    let repeat = request(&app, "PATCH", &notification_note_path, &owner,
        json!({"title":"Mentions","content":format!("Hi @{member_name} `@{owner_name}` [ref](https://example.test/@outsider_name)"),"version":2})).await;
    assert_eq!(repeat.0, StatusCode::OK, "{}", repeat.1);
    let count_for = |rows: &[toasty::stmt::Value]| {
        rows.first().is_some_and(|row| matches!(row, toasty::stmt::Value::Record(record) if matches!(record.fields.first(), Some(toasty::stmt::Value::I64(1)))))
    };
    let count = toasty::sql::query(
        "SELECT count(*) AS n FROM notification_events WHERE note_id = $1 AND user_id = $2",
    )
    .bind(notification_note_id)
    .bind(member.as_str())
    .exec(&mut db)
    .await?;
    assert!(
        count_for(&count),
        "re-saving an unchanged mention created a duplicate"
    );
    let channel_target = request(
        &app,
        "PATCH",
        &notification_note_path,
        &owner,
        json!({"title":"Mentions","content":format!("Hi @{member_name} @{outsider_name} @channel"),"version":3}),
    )
    .await;
    assert_eq!(channel_target.0, StatusCode::OK, "{}", channel_target.1);
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &bot).await?,
        0,
        "bots do not get inbox notifications"
    );
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &member).await?,
        1
    );
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &outsider).await?,
        1
    );
    let channel_removed = request(
        &app,
        "PATCH",
        &notification_note_path,
        &owner,
        json!({"title":"Mentions","content":format!("Hi @{member_name} @{outsider_name}"),"version":4}),
    )
    .await;
    assert_eq!(channel_removed.0, StatusCode::OK, "{}", channel_removed.1);
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &member).await?,
        1
    );
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &outsider).await?,
        1
    );
    let ids = toasty::sql::query(
        "SELECT id FROM notification_events WHERE note_id = $1 AND user_id = $2",
    )
    .bind(notification_note_id)
    .bind(member.as_str())
    .exec(&mut db)
    .await?;
    let notification_id = match ids.first() {
        Some(toasty::stmt::Value::Record(row)) => match row.fields.first() {
            Some(toasty::stmt::Value::String(id)) => id.clone(),
            other => panic!("unexpected id: {other:?}"),
        },
        other => panic!("unexpected notification rows: {other:?}"),
    };
    let (context_status, context) = request(
        &app,
        "GET",
        &format!("/api/notifications/{notification_id}/context"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(context_status, StatusCode::OK, "{context}");
    assert_eq!(context["note"]["id"], notification_note_id);
    assert_eq!(context["message"], Value::Null);
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/api/notifications/{notification_id}/context"),
            &outsider,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (read_status, read) = request(
        &app,
        "PATCH",
        &format!("/api/notifications/{notification_id}/read"),
        &member,
        Value::Null,
    )
    .await;
    assert_eq!(read_status, StatusCode::OK, "{read}");
    assert_eq!(read["data"]["is_read"], true);
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/notifications/{notification_id}/read"),
            &outsider,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let removed = request(
        &app,
        "PATCH",
        &notification_note_path,
        &owner,
        json!({"title":"Mentions","content":"mention removed","version":5}),
    )
    .await;
    assert_eq!(removed.0, StatusCode::OK, "{}", removed.1);
    let readded = request(
        &app,
        "PATCH",
        &notification_note_path,
        &owner,
        json!({"title":"Mentions","content":format!("again @{member_name} @{outsider_name}"),"version":6}),
    )
    .await;
    assert_eq!(readded.0, StatusCode::OK, "{}", readded.1);
    let count = toasty::sql::query(
        "SELECT count(*) AS n FROM notification_events WHERE note_id = $1 AND user_id = $2",
    )
    .bind(notification_note_id)
    .bind(member.as_str())
    .exec(&mut db)
    .await?;
    assert!(
        matches!(count.first(), Some(toasty::stmt::Value::Record(record)) if matches!(record.fields.first(), Some(toasty::stmt::Value::I64(2)))),
        "removing and re-adding a mention should create a new notification"
    );
    assert_eq!(
        note_notification_count(&mut db, notification_note_id, &outsider).await?,
        2
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            &notification_note_path,
            &owner,
            json!({"version":7})
        )
        .await
        .0,
        StatusCode::OK
    );
    let deleted_context = request(
        &app,
        "GET",
        &format!("/api/notifications/{notification_id}/context"),
        &member,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(deleted_context["note"], Value::Null);
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{notification_note_path}/restore"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let restored_context = request(
        &app,
        "GET",
        &format!("/api/notifications/{notification_id}/context"),
        &member,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(restored_context["note"]["id"], notification_note_id);

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
