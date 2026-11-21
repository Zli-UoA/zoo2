//! The reply gate of `POST /api/messages` (a reply to a thread that tracks participants needs a
//! join): the first reply, a member who never joined, the lock-wait race with a stored retry,
//! and the Apps API bot exemption. Each test has its own schema, so it runs in isolation.
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

async fn send(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", user)
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
async fn call(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    send(app, user, method, path, body, &[]).await
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

struct World {
    app: Router,
    db: toasty::Db,
    fixture: thread_db::ThreadDb,
    users: Vec<String>,
    channel: String,
}

/// Three channel members (users[0] owns the channel); the rollout is `ready`.
async fn world() -> anyhow::Result<World> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
    let mut db = init_db(&config.database_url).await?;
    toasty::sql::statement("UPDATE thread_rollout SET phase='ready'")
        .exec(&mut db)
        .await?;
    let app = create_router(AppState::new(db.clone(), config));
    let users: Vec<_> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
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
    for user in &users[1..] {
        ok(
            &app,
            user,
            "POST",
            &format!("/api/channels/{channel}/join"),
            json!({}),
        )
        .await?;
    }
    Ok(World {
        app,
        db,
        fixture,
        users,
        channel,
    })
}

impl World {
    fn reply(&self, parent: &str, content: &str) -> Value {
        json!({"id":uuid::Uuid::new_v4().to_string(),"channel_id":self.channel,"parent_id":parent,"content":content})
    }
    async fn root(&self) -> anyhow::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        ok(
            &self.app,
            &self.users[0],
            "POST",
            "/api/messages",
            json!({"id":id,"channel_id":self.channel,"content":"Root"}),
        )
        .await?;
        Ok(id)
    }
    async fn thread(&self, user: &str, root: &str) -> anyhow::Result<Value> {
        Ok(ok(
            &self.app,
            user,
            "GET",
            &format!("/api/threads/{root}"),
            Value::Null,
        )
        .await?["data"]
            .clone())
    }
}

#[tokio::test]
async fn the_first_reply_creates_the_thread_without_a_join() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    // No participant rows yet: nothing to join.
    let before = w.thread(&w.users[1], &root).await?;
    assert_eq!(before["tracked"], false);
    assert_eq!(before["membership"], "none");
    let (status, _) = call(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    // The thread now tracks its participants (the parent author and the replier).
    let after = w.thread(&w.users[1], &root).await?;
    assert_eq!(after["tracked"], true);
    assert_eq!(after["membership"], "joined");
    assert_eq!(after["participant_count"], 2);
    w.fixture.cleanup().await
}

#[tokio::test]
async fn a_member_who_never_joined_is_refused_with_the_join_code() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    // users[2] is a channel member who has never joined this thread.
    let (status, body) = call(
        &w.app,
        &w.users[2],
        "POST",
        "/api/messages",
        w.reply(&root, "late"),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Clients decide on the code; clients released before it existed match the message.
    assert_eq!(body["code"], "THREAD_JOIN_REQUIRED");
    assert_eq!(body["error"], "Join the thread before replying");
    // After joining, the same reply goes through.
    ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("/api/threads/{root}/join"),
        json!({}),
    )
    .await?;
    let (status, _) = call(
        &w.app,
        &w.users[2],
        "POST",
        "/api/messages",
        w.reply(&root, "late"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    w.fixture.cleanup().await
}

/// A request that passed the idempotency check, then waited for the thread lock while the same
/// post was stored and its author left, is a retry of the stored post: it returns that post
/// instead of 403, and does not join the author again.
#[tokio::test]
async fn a_retry_that_waited_for_the_lock_returns_the_stored_post() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    let payload = w.reply(&root, "retry me");
    let id = payload["id"].as_str().unwrap().to_owned();
    let app = w.app.clone();
    let author = w.users[1].clone();
    let retry_payload = payload.clone();
    // The retry passes the pre-check (nothing is stored yet), then waits up to a second at the
    // test gate, which nobody notifies.
    let retry = tokio::spawn(async move {
        send(
            &app,
            &author,
            "POST",
            "/api/messages",
            retry_payload,
            &[("X-Test-Gate-Wait", "never-notified")],
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        payload.clone(),
    )
    .await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("/api/threads/{root}/leave"),
        json!({}),
    )
    .await?;
    let (status, body) = retry.await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], id);
    // The leave stays: returning the stored post is not a new reply.
    assert_eq!(w.thread(&w.users[1], &root).await?["membership"], "left");
    w.fixture.cleanup().await
}

#[tokio::test]
async fn an_installed_app_bot_is_not_gated() -> anyhow::Result<()> {
    let mut w = world().await?;
    let root = w.root().await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    // An installed app's bot user that is a member of the channel (it has no way to join a thread).
    let bot = uuid::Uuid::new_v4().to_string();
    ok(
        &w.app,
        &bot,
        "POST",
        "/api/users",
        json!({"id":bot,"user_name":bot,"display_name":bot}),
    )
    .await?;
    let app_id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("UPDATE users SET is_bot=true WHERE id=$1")
        .bind(bot.as_str())
        .exec(&mut w.db)
        .await?;
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now() AT TIME ZONE 'UTC')",
    )
    .bind(bot.as_str())
    .bind(w.channel.as_str())
    .exec(&mut w.db)
    .await?;
    toasty::sql::statement(
        "INSERT INTO apps(id,owner_id,bot_user_id,name,status) VALUES($1,$2,$3,'gate','installed')",
    )
    .bind(app_id.as_str())
    .bind(w.users[0].as_str())
    .bind(bot.as_str())
    .exec(&mut w.db)
    .await?;
    toasty::sql::statement(
        "INSERT INTO app_installations(id,app_id,revision,active) VALUES($1,$1,1,true)",
    )
    .bind(app_id.as_str())
    .exec(&mut w.db)
    .await?;
    let (status, body) = call(
        &w.app,
        &bot,
        "POST",
        "/api/messages",
        w.reply(&root, "from the bot"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    w.fixture.cleanup().await
}

/// 参加者の一覧と人数に載るのは、いまチャンネルにいる人だけ。チャンネルを退出しても、スレッドの
/// 参加行は残るが、一覧は共有チャンネルのユーザーだけを見せる既存の可視性を超えない。
#[tokio::test]
async fn a_participant_who_left_the_channel_is_not_listed() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("/api/threads/{root}/join"),
        json!({}),
    )
    .await?;
    let listed = |thread: &Value| -> Vec<String> {
        thread["participants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = w.thread(&w.users[0], &root).await?;
    assert_eq!(before["participant_count"], 3);
    assert!(listed(&before).contains(&w.users[2]));
    ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("/api/channels/{}/leave", w.channel),
        json!({}),
    )
    .await?;
    let after = w.thread(&w.users[0], &root).await?;
    assert_eq!(after["participant_count"], 2);
    assert!(!listed(&after).contains(&w.users[2]));
    // The thread still tracks participants (the row of the member who left is kept).
    assert_eq!(after["tracked"], true);
    w.fixture.cleanup().await
}
