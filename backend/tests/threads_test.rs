use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

async fn request(
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
                .header("content-type", "application/json")
                .header("X-Dev-User", user)
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

async fn ok(
    app: &Router,
    method: &str,
    user: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<Value> {
    let (status, result) = request(app, method, user, path, body).await?;
    assert!(status.is_success(), "{status}: {result}");
    Ok(result)
}

#[tokio::test]
async fn thread_channels_are_public_joinable_and_archivable() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let charlie = uuid::Uuid::new_v4().to_string();
    let parent = uuid::Uuid::new_v4().to_string();

    ok(
        &app,
        "POST",
        &alice,
        "/api/users",
        json!({"id":alice,"user_name":format!("alice_{}",uuid::Uuid::new_v4().simple()),"display_name":"Alice"}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &charlie,
        "/api/users",
        json!({"id":charlie,"user_name":format!("charlie_{}",uuid::Uuid::new_v4().simple()),"display_name":"Charlie"}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &bob,
        "/api/users",
        json!({"id":bob,"user_name":format!("bob_{}",uuid::Uuid::new_v4().simple()),"display_name":"Bob"}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &alice,
        "/api/channels",
        json!({"id":parent,"channel_name":format!("thread-test-{}",uuid::Uuid::new_v4().simple()),"is_private":false}),
    )
    .await?;

    let parent_post = ok(
        &app,
        "POST",
        &alice,
        "/api/messages",
        json!({"channel_id":parent,"content":"Parent channel post"}),
    )
    .await?["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Simulate an old backend write during the rolling upgrade. The migration trigger
    // registers the pre-existing root so the new backend can preserve its old reply pane.
    let legacy_reply_id = uuid::Uuid::new_v4().to_string();
    let mut test_db = db.clone();
    let mut tx = test_db.transaction().await?;
    toasty::sql::statement(
        "INSERT INTO messages(id,user_id,channel_id,content,parent_id,created_at) \
         VALUES($1,$2,$3,'Legacy reply',$4,now() AT TIME ZONE 'UTC')",
    )
    .bind(legacy_reply_id.as_str())
    .bind(bob.as_str())
    .bind(parent.as_str())
    .bind(parent_post.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;

    let legacy_reply = ok(
        &app,
        "POST",
        &charlie,
        "/api/messages",
        json!({"channel_id":parent,"content":"Reply during rollout","parent_id":parent_post}),
    )
    .await?;
    assert_eq!(legacy_reply["data"]["parent_id"], parent_post);
    let legacy_threads = ok(
        &app,
        "GET",
        &alice,
        &format!("/api/threads?channel_id={parent}&archived=false"),
        Value::Null,
    )
    .await?;
    assert_eq!(legacy_threads["items"][0]["id"], parent_post);
    assert_eq!(legacy_threads["items"][0]["legacy"], true);
    let archived_legacy = ok(
        &app,
        "POST",
        &alice,
        &format!("/api/threads/{parent_post}/archive"),
        json!({}),
    )
    .await?;
    assert!(archived_legacy["data"]["thread_archive_at"].is_string());
    ok(
        &app,
        "POST",
        &alice,
        &format!("/api/threads/{parent_post}/open"),
        json!({}),
    )
    .await?;

    let created = ok(
        &app,
        "POST",
        &alice,
        &format!("/api/channels/{parent}/threads"),
        json!({"title":"Release planning","initial_post":"Let's plan the release."}),
    )
    .await?;
    let thread = created["data"]["channel_id"].as_str().unwrap().to_owned();
    let first_post = created["data"]["first_message_id"].as_str().unwrap();
    assert_eq!(created["data"]["parent_channel_id"], parent);

    let (archive_status, _) = request(
        &app,
        "POST",
        &alice,
        &format!("/api/channels/{thread}/archive"),
        json!({}),
    )
    .await?;
    assert_eq!(archive_status, StatusCode::BAD_REQUEST);
    let (update_status, _) = request(
        &app,
        "PATCH",
        &alice,
        &format!("/api/channels/{thread}"),
        json!({"is_private":true}),
    )
    .await?;
    assert_eq!(update_status, StatusCode::BAD_REQUEST);

    let listed = ok(
        &app,
        "GET",
        &bob,
        &format!("/api/channels/{parent}/threads?archived=false"),
        Value::Null,
    )
    .await?;
    assert_eq!(listed["items"][0]["channel_id"], thread);
    assert_eq!(listed["items"][0]["title"], "Release planning");
    assert_eq!(listed["items"][0]["is_member"], false);

    let (cross_channel_reply_status, _) = request(
        &app,
        "POST",
        &alice,
        "/api/messages",
        json!({"channel_id":thread,"content":"Cross-channel reply","reply_to_message_id":parent_post}),
    )
    .await?;
    assert_eq!(cross_channel_reply_status, StatusCode::NOT_FOUND);

    ok(
        &app,
        "POST",
        &bob,
        &format!("/api/threads/{thread}/join"),
        json!({}),
    )
    .await?;
    let reply = ok(
        &app,
        "POST",
        &bob,
        "/api/messages",
        json!({"channel_id":thread,"content":"I will draft the timeline.","reply_to_message_id":first_post}),
    )
    .await?;
    assert!(reply["data"]["parent_id"].is_null());
    assert_eq!(reply["data"]["reply_to_message_id"], first_post);

    ok(
        &app,
        "POST",
        &alice,
        &format!("/api/threads/{thread}/archive"),
        json!({}),
    )
    .await?;
    let archived = ok(
        &app,
        "GET",
        &bob,
        &format!("/api/channels/{parent}/threads?archived=true"),
        Value::Null,
    )
    .await?;
    assert_eq!(archived["items"][0]["channel_id"], thread);
    assert!(archived["items"][0]["archived_at"].is_string());

    // Posting remains allowed in an archived thread and reopens it atomically.
    ok(
        &app,
        "POST",
        &bob,
        "/api/messages",
        json!({"channel_id":thread,"content":"One more detail.","reply_to_message_id":first_post}),
    )
    .await?;
    let active = ok(
        &app,
        "GET",
        &alice,
        &format!("/api/channels/{parent}/threads?archived=false"),
        Value::Null,
    )
    .await?;
    assert_eq!(active["items"][0]["channel_id"], thread);
    assert!(active["items"][0]["archived_at"].is_null());

    ok(
        &app,
        "POST",
        &bob,
        &format!("/api/threads/{thread}/archive"),
        json!({}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &bob,
        &format!("/api/threads/{thread}/open"),
        json!({}),
    )
    .await?;
    let manually_opened = ok(
        &app,
        "GET",
        &alice,
        &format!("/api/channels/{parent}/threads?archived=false"),
        Value::Null,
    )
    .await?;
    assert!(manually_opened["items"][0]["archived_at"].is_null());

    ok(
        &app,
        "POST",
        &alice,
        &format!("/api/threads/{thread}/archive"),
        json!({}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &charlie,
        &format!("/api/threads/{thread}/join"),
        json!({}),
    )
    .await?;
    let joined_archived = ok(
        &app,
        "GET",
        &alice,
        &format!("/api/channels/{parent}/threads?archived=true"),
        Value::Null,
    )
    .await?;
    assert_eq!(joined_archived["items"][0]["channel_id"], thread);
    assert!(joined_archived["items"][0]["archived_at"].is_string());
    assert!(joined_archived["items"][0]["is_member"].as_bool().unwrap());

    // Posting to an archived thread adds the poster as a member and reopens it.
    ok(
        &app,
        "POST",
        &charlie,
        "/api/messages",
        json!({"channel_id":thread,"content":"Reply after joining"}),
    )
    .await?;
    let active_after_post = ok(
        &app,
        "GET",
        &alice,
        &format!("/api/channels/{parent}/threads?archived=false"),
        Value::Null,
    )
    .await?;
    assert_eq!(active_after_post["items"][0]["channel_id"], thread);
    assert!(active_after_post["items"][0]["archived_at"].is_null());

    // Old nested-thread writes are rejected; a reply uses the explicit link field.
    let (status, _) = request(
        &app,
        "POST",
        &bob,
        "/api/messages",
        json!({"channel_id":thread,"content":"Nested thread","parent_id":first_post}),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    ok(
        &app,
        "DELETE",
        &bob,
        &format!("/api/threads/{thread}/leave"),
        Value::Null,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn private_parent_members_can_read_but_must_join_before_posting() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let parent = uuid::Uuid::new_v4().to_string();
    ok(&app, "POST", &alice, "/api/users", json!({"id":alice,"user_name":format!("private_thread_alice_{}",uuid::Uuid::new_v4().simple()),"display_name":"Alice"})).await?;
    ok(&app, "POST", &bob, "/api/users", json!({"id":bob,"user_name":format!("private_thread_bob_{}",uuid::Uuid::new_v4().simple()),"display_name":"Bob"})).await?;
    ok(&app, "POST", &alice, "/api/channels", json!({"id":parent,"channel_name":format!("private-thread-{}",uuid::Uuid::new_v4().simple()),"is_private":true})).await?;
    ok(
        &app,
        "POST",
        &alice,
        &format!("/api/channels/{parent}/users"),
        json!({"user_id":bob}),
    )
    .await?;
    let created = ok(
        &app,
        "POST",
        &alice,
        &format!("/api/channels/{parent}/threads"),
        json!({"title":"Private discussion","initial_post":"Read access follows the parent."}),
    )
    .await?;
    let thread = created["data"]["channel_id"].as_str().unwrap();

    let (status, messages) = request(
        &app,
        "GET",
        &bob,
        &format!("/api/messages?channel_id={thread}"),
        Value::Null,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{messages}");
    assert_eq!(messages.as_array().unwrap().len(), 1);
    let (status, _) = request(
        &app,
        "POST",
        &bob,
        "/api/messages",
        json!({"channel_id":thread,"content":"Not joined yet"}),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    ok(
        &app,
        "POST",
        &bob,
        &format!("/api/threads/{thread}/join"),
        json!({}),
    )
    .await?;
    ok(
        &app,
        "POST",
        &bob,
        "/api/messages",
        json!({"channel_id":thread,"content":"Joined and posting"}),
    )
    .await?;
    Ok(())
}
