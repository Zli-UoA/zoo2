//! The common inbox is authoritative; legacy stores are rolling-deployment projections.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    models::{Channel, Message, Notification, User, now_utc_millis},
    routes::create_router,
    state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn setup() -> anyhow::Result<(Router, toasty::Db, String, String, String, String)> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let owner = Uuid::new_v4().to_string();
    let actor = Uuid::new_v4().to_string();
    let now = now_utc_millis();
    for id in [&owner, &actor] {
        User::create()
            .id(id.as_str())
            .user_name(format!("inbox_{id}"))
            .display_name("Inbox user")
            .is_bot(false)
            .created_at(now)
            .exec(&mut db)
            .await?;
    }
    let channel = Uuid::new_v4().to_string();
    Channel::create()
        .id(channel.as_str())
        .channel_name(format!("inbox-{channel}"))
        .created_user_id(owner.as_str())
        .is_private(false)
        .is_nsfw(false)
        .is_dm(false)
        .created_at(now)
        .exec(&mut db)
        .await?;
    let message = Uuid::new_v4().to_string();
    Message::create()
        .id(message.as_str())
        .channel_id(channel.as_str())
        .user_id(owner.as_str())
        .content("Inbox preview")
        .apub(false)
        .created_at(now)
        .exec(&mut db)
        .await?;
    let app = create_router(AppState::new(db.clone(), config));
    Ok((app, db, owner, actor, channel, message))
}
async fn sql(db: &mut toasty::Db, query: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut tx = db.transaction().await?;
    let rows = backend::apps::query(&mut tx, query, params).await?;
    tx.commit().await?;
    Ok(rows)
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    user: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("X-Dev-User", user)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    ))
}
async fn channel_notice(
    db: &mut toasty::Db,
    owner: &str,
    actor: &str,
    channel: &str,
    message: &str,
) -> anyhow::Result<String> {
    let id = Uuid::new_v4().to_string();
    Notification::create()
        .id(id.as_str())
        .user_id(owner)
        .actor_id(actor)
        .channel_id(channel)
        .message_id(message)
        .notification_type("mention")
        .is_read(false)
        .created_at(now_utc_millis())
        .exec(db)
        .await?;
    Ok(id)
}
async fn remote_notice(db: &mut toasty::Db, owner: &str) -> anyhow::Result<String> {
    let id = Uuid::new_v4().to_string();
    sql(db, "INSERT INTO notification_events(id,user_id,source,actor_id,activity_id,notification_type,reaction) VALUES($1,$2,'activitypub','https://remote.example/actor',$1,'like','🎉') RETURNING json_build_object('id',id)::text", &[&id, owner]).await?;
    Ok(id)
}

#[tokio::test]
async fn shared_read_all_updates_both_sources_and_keeps_owner_and_channel_scope()
-> anyhow::Result<()> {
    let (app, mut db, owner, actor, channel, message) = setup().await?;
    let local = channel_notice(&mut db, &owner, &actor, &channel, &message).await?;
    let remote = remote_notice(&mut db, &owner).await?;
    let foreign = remote_notice(&mut db, &actor).await?;
    let (status, scoped) = call(
        &app,
        "POST",
        "/api/notifications/read-all",
        &owner,
        json!({"channel_id":channel}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scoped["data"]["updated_count"], 1);
    assert!(scoped["txid"].is_string());
    let rows = sql(&mut db, "SELECT json_build_object('id',id,'read',is_read)::text FROM notification_events WHERE id IN ($1,$2,$3) ORDER BY id", &[&local,&remote,&foreign]).await?;
    for row in rows {
        assert_eq!(row["read"], row["id"] == local);
    }
    let (status, all) = call(
        &app,
        "POST",
        "/api/notifications/read-all",
        &owner,
        json!({}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["data"]["updated_count"], 1);
    // Only the federated row changed: there is no channel Shape transaction to wait for.
    assert!(all["txid"].is_null());
    let (_, again) = call(
        &app,
        "POST",
        "/api/notifications/read-all",
        &owner,
        json!({}),
    )
    .await?;
    assert_eq!(again["data"]["updated_count"], 0);
    assert!(again["txid"].is_null());
    let (_, remote_page) = call(
        &app,
        "GET",
        "/api/activitypub/notifications",
        &owner,
        json!({}),
    )
    .await?;
    assert_eq!(remote_page["unread_count"], 0);
    assert_eq!(remote_page["items"][0]["reaction"], "🎉");
    assert_eq!(remote_page["items"][0]["is_read"], true);
    let (context_status, context) = call(
        &app,
        "GET",
        &format!("/api/notifications/{remote}/context"),
        &owner,
        json!({}),
    )
    .await?;
    assert_eq!(context_status, StatusCode::NOT_FOUND);
    assert!(context.get("message").is_none());
    let rows = sql(
        &mut db,
        "SELECT json_build_object('read',is_read)::text FROM notification_events WHERE id=$1",
        &[&foreign],
    )
    .await?;
    assert_eq!(rows[0]["read"], false);
    Ok(())
}

#[tokio::test]
async fn old_and_new_writers_share_metadata_read_state_and_deletion_without_trigger_loops()
-> anyhow::Result<()> {
    let (_, mut db, owner, actor, channel, message) = setup().await?;
    let local = channel_notice(&mut db, &owner, &actor, &channel, &message).await?;
    let remote = remote_notice(&mut db, &owner).await?;
    // An old process can create new events after the migration has been applied.
    let old_remote = Uuid::new_v4().to_string();
    sql(&mut db, "INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind,created_at) VALUES($1,$2,'https://remote.example/old',$1,'reply','2026-11-02T12:00:00.123456Z') RETURNING json_build_object('id',id)::text", &[&old_remote,&owner]).await?;
    let rows = sql(&mut db, "SELECT json_build_object('source',source,'time',created_at::text)::text FROM notification_events WHERE id=$1", &[&old_remote]).await?;
    assert_eq!(rows[0]["source"], "activitypub");
    assert_eq!(rows[0]["time"], "2026-11-02 12:00:00.123456");
    sql(&mut db, "UPDATE apub_notifications SET reaction='🦊',emoji='{\"name\":\":fox:\"}',is_read=true WHERE id=$1 RETURNING json_build_object('id',id)::text", &[&remote]).await?;
    let rows = sql(&mut db, "SELECT json_build_object('reaction',reaction,'emoji',emoji,'read',is_read)::text FROM notification_events WHERE id=$1", &[&remote]).await?;
    assert_eq!(
        rows[0],
        json!({"reaction":"🦊","emoji":{"name":":fox:"},"read":true})
    );
    sql(&mut db, "UPDATE notifications SET is_read=true WHERE id=$1 RETURNING json_build_object('id',id)::text", &[&local]).await?;
    assert!(
        Notification::get_by_id(&mut db, local.as_str())
            .await?
            .is_read
    );
    sql(
        &mut db,
        "DELETE FROM notification_events WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&remote],
    )
    .await?;
    assert!(
        sql(
            &mut db,
            "SELECT row_to_json(n)::text FROM apub_notifications n WHERE id=$1",
            &[&remote]
        )
        .await?
        .is_empty()
    );
    sql(
        &mut db,
        "DELETE FROM apub_notifications WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&old_remote],
    )
    .await?;
    assert!(
        sql(
            &mut db,
            "SELECT row_to_json(n)::text FROM notification_events n WHERE id=$1",
            &[&old_remote]
        )
        .await?
        .is_empty()
    );
    // Existing channel/message cascade behavior still removes the shared event and projection.
    sql(
        &mut db,
        "DELETE FROM messages WHERE id=$1 RETURNING json_build_object('id',id)::text",
        &[&message],
    )
    .await?;
    assert!(
        sql(
            &mut db,
            "SELECT row_to_json(n)::text FROM notification_events n WHERE id=$1",
            &[&local]
        )
        .await?
        .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn shared_selection_accepts_both_sources_but_never_reads_other_users_or_unselected_rows()
-> anyhow::Result<()> {
    let (app, mut db, owner, actor, channel, message) = setup().await?;
    let local = channel_notice(&mut db, &owner, &actor, &channel, &message).await?;
    let remote = remote_notice(&mut db, &owner).await?;
    let unselected = remote_notice(&mut db, &owner).await?;
    let foreign = remote_notice(&mut db, &actor).await?;
    let (status, _) = call(&app, "POST", "/api/notifications/read", &owner, json!({})).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, empty) = call(
        &app,
        "POST",
        "/api/notifications/read",
        &owner,
        json!({"ids":[]}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["data"]["updated_count"], 0);
    assert!(empty["txid"].is_null());
    let (status, selected) = call(
        &app,
        "POST",
        "/api/notifications/read",
        &owner,
        json!({"ids":[local, remote, foreign, local]}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(selected["data"]["updated_count"], 2);
    let rows = sql(&mut db, "SELECT json_build_object('id',id,'read',is_read)::text FROM notification_events WHERE id IN ($1,$2,$3,$4)", &[&local,&remote,&unselected,&foreign]).await?;
    for row in rows {
        assert_eq!(row["read"], row["id"] == local || row["id"] == remote);
    }
    let (status, _) = call(
        &app,
        "POST",
        "/api/notifications/read",
        &owner,
        json!({"ids":vec![unselected;501]}),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn electric_stream_reads_the_common_store_without_changing_the_client_row_contract()
-> anyhow::Result<()> {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    let (_, db, owner, _, _, _) = setup().await?;
    let electric = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shape"))
        .and(query_param("table", "notification_events"))
        .and(query_param(
            "columns",
            "id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at,display_data",
        ))
        .and(query_param("params[1]", owner.as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("electric-handle", "common-inbox")
                .set_body_json(json!([])),
        )
        .expect(1)
        .mount(&electric)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.electric_url = electric.uri();
    let app = create_router(AppState::new(db, config));
    let (status, _) = call(
        &app,
        "GET",
        "/api/shapes/notifications?offset=-1&table=apub_notifications&columns=emoji",
        &owner,
        json!({}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let requests = electric.received_requests().await.unwrap();
    let params: std::collections::HashMap<_, _> =
        requests[0].url.query_pairs().into_owned().collect();
    let filter = &params["where"];
    assert!(filter.contains("source = 'channel'"));
    assert!(filter.contains("user_id = $1"));
    assert!(filter.contains("message_deleted = false"));
    assert!(!filter.contains("FROM messages"));
    assert!(filter.contains("deleted_at IS NULL"));
    assert!(filter.contains("channel_user"));
    electric.verify().await;
    Ok(())
}
