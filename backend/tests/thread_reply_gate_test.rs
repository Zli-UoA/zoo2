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
use backend::{
    auth::AuthUser,
    db::init_db,
    routes::{create_router, threads::read_participants},
    state::AppState,
};
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
/// Like `send`, with the `X-Test-Fallback` header of the answer (the branch that produced it).
async fn send_with_fallback(
    app: &Router,
    user: &str,
    path: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> anyhow::Result<(StatusCode, Value, Option<String>)> {
    let mut request = Request::builder()
        .method("POST")
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
    let fallback = response
        .headers()
        .get("X-Test-Fallback")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?, fallback))
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
    world_in("ready").await
}

/// The same, with the staged migration in `phase` (`collecting`, `normalizing` or `ready`).
async fn world_in(phase: &str) -> anyhow::Result<World> {
    world_with(phase, |_| {}).await
}

/// The same, with the server's configuration adjusted by `adjust` before it starts.
async fn world_with(
    phase: &str,
    adjust: impl FnOnce(&mut backend::config::Config),
) -> anyhow::Result<World> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let mut config = fixture.config.clone();
    adjust(&mut config);
    let mut db = init_db(&config.database_url).await?;
    toasty::sql::statement(&format!("UPDATE thread_rollout SET phase='{phase}'"))
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
    /// `count` more channel members, all participants of the thread `root` (live ones).
    async fn crowd(&self, root: &str, count: i32) -> anyhow::Result<()> {
        let (tag, channel) = (uuid::Uuid::new_v4().simple().to_string(), &self.channel);
        let mut db = self.db.clone();
        for sql in [
            format!(
                "INSERT INTO users(id,user_name,display_name) SELECT '{tag}-'||n,'{tag}-'||n,'U' FROM generate_series(1,{count}) n"
            ),
            format!(
                "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT '{tag}-'||n,'{channel}',now() FROM generate_series(1,{count}) n"
            ),
            format!(
                "INSERT INTO thread_participants(thread_id,user_id) SELECT '{root}','{tag}-'||n FROM generate_series(1,{count}) n"
            ),
        ] {
            toasty::sql::statement(&sql).exec(&mut db).await?;
        }
        Ok(())
    }
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
///
/// The order is fixed by the test, not by time: the retry signals that it passed the first
/// idempotency check and waits at a gate that the test releases once the post and the leave are
/// committed. The gate's timeout only keeps a broken run from hanging.
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
    let (reached, release) = (
        format!("reached-{}", uuid::Uuid::new_v4().simple()),
        format!("release-{}", uuid::Uuid::new_v4().simple()),
    );
    let app = w.app.clone();
    let author = w.users[1].clone();
    let retry_payload = payload.clone();
    let (retry_reached, retry_release) = (reached.clone(), release.clone());
    let retry = tokio::spawn(async move {
        send_with_fallback(
            &app,
            &author,
            "/api/messages",
            retry_payload,
            &[
                ("X-Test-Gate-Wait", retry_release.as_str()),
                ("X-Test-Gate-Reached", retry_reached.as_str()),
                ("X-Test-Gate-Wait-Ms", "20000"),
            ],
        )
        .await
    });
    // The retry has passed its first idempotency check (nothing was stored yet) and waits.
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        backend::routes::messages::test_gate(&reached).notified(),
    )
    .await?;
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
    // Both are committed: let the retry go on to the thread lock.
    backend::routes::messages::test_gate(&release).notify_one();
    let (status, body, fallback) = retry.await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], id);
    // The answer came from the gate's recheck, not from the ordinary path.
    assert_eq!(fallback.as_deref(), Some("thread-join-retry"));
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

/// 参加者を表示しないクライアントは、`participants=none` で人数と一覧（O(P) の読み取り）を省く。
/// `tracked` と参加状態は、省いても返る。
#[tokio::test]
async fn participants_can_be_left_out_of_the_answer() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    let path = format!("/api/threads/{root}");
    // Untracked: no participant rows yet.
    let (_, empty) = call(
        &w.app,
        &w.users[1],
        "GET",
        &format!("{path}?participants=none"),
        Value::Null,
    )
    .await?;
    assert_eq!(empty["data"]["tracked"], false);
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    let lean = ok(
        &w.app,
        &w.users[2],
        "GET",
        &format!("{path}?participants=none"),
        Value::Null,
    )
    .await?;
    assert_eq!(lean["data"]["tracked"], true);
    assert_eq!(lean["data"]["membership"], "none");
    assert!(lean["data"].get("participants").is_none());
    assert!(lean["data"].get("participant_count").is_none());
    // A write leaves them out too, and still answers the state it produced.
    let joined = ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("{path}/join?participants=none"),
        json!({}),
    )
    .await?;
    assert_eq!(joined["data"]["membership"], "joined");
    assert_eq!(joined["data"]["tracked"], true);
    assert!(joined["data"].get("participants").is_none());
    // Without the parameter, or with another value, the answer is the full one.
    for query in ["", "?participants=list"] {
        let full = ok(
            &w.app,
            &w.users[2],
            "GET",
            &format!("{path}{query}"),
            Value::Null,
        )
        .await?;
        assert_eq!(full["data"]["participant_count"], 3);
        assert_eq!(full["data"]["participants"].as_array().unwrap().len(), 3);
    }
    w.fixture.cleanup().await
}

/// 返信がまだないメッセージにも、最初の返信の前に参加できる（Web は、入力欄の代わりに参加ボタンを出す）。
/// 参加しただけでは、スレッド一覧に出ない。参加後は参加行があるので、親投稿者も参加が要る。
#[tokio::test]
async fn a_message_without_replies_can_be_joined_before_the_first_reply() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    let path = format!("/api/threads/{root}");
    let joined = ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("{path}/join"),
        json!({}),
    )
    .await?;
    assert_eq!(joined["data"]["membership"], "joined");
    assert_eq!(joined["data"]["tracked"], true);
    assert_eq!(joined["data"]["participant_count"], 1);
    // Joining alone creates no thread: it stays out of the index until a reply exists.
    let index = ok(&w.app, &w.users[1], "GET", "/api/threads", Value::Null).await?;
    assert!(
        !index["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == root.as_str())
    );
    // The thread now has a participant row, so the author needs a join too.
    let (status, body) = call(
        &w.app,
        &w.users[0],
        "POST",
        "/api/messages",
        w.reply(&root, "author"),
    )
    .await?;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::FORBIDDEN, &json!("THREAD_JOIN_REQUIRED"))
    );
    // The first reply goes through, and the parent author joins with it.
    let (status, _) = call(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let after = w.thread(&w.users[1], &root).await?;
    assert_eq!(after["participant_count"], 2);
    let index = ok(&w.app, &w.users[1], "GET", "/api/threads", Value::Null).await?;
    assert!(
        index["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == root.as_str())
    );
    w.fixture.cleanup().await
}

/// 投稿権限の確認は、参加の確認より先。管理者のみ投稿できるチャンネルの非管理者は、参加しているかどうかに
/// よらず、投稿権限の 403（`POSTING_RESTRICTED`）になり、参加を求められない（参加しても送れないため）。
#[tokio::test]
async fn the_posting_policy_is_checked_before_the_join() -> anyhow::Result<()> {
    let mut w = world().await?;
    let root = w.root().await?;
    // users[1] replies first (the thread now tracks participants), then the channel becomes managers-only.
    ok(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    toasty::sql::statement("UPDATE channels SET posting_policy='managers' WHERE id=$1")
        .bind(w.channel.as_str())
        .exec(&mut w.db)
        .await?;
    // users[1] is joined and users[2] is not: neither is the creator or an administrator.
    for user in [&w.users[1], &w.users[2]] {
        let (status, body) =
            call(&w.app, user, "POST", "/api/messages", w.reply(&root, "x")).await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "POSTING_RESTRICTED", "{body}");
        assert_eq!(body["error"], "Only channel managers may post here");
    }
    // The creator still posts: as the author of the root, they joined with the first reply.
    let (status, body) = call(
        &w.app,
        &w.users[0],
        "POST",
        "/api/messages",
        w.reply(&root, "creator"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    w.fixture.cleanup().await
}

/// 参加して退出しただけでも、参加行は残る（`tracked`）。そのスレッドへの最初の返信も、全員が参加してから。
#[tokio::test]
async fn joining_and_leaving_keeps_the_thread_tracked() -> anyhow::Result<()> {
    let w = world().await?;
    let root = w.root().await?;
    let path = format!("/api/threads/{root}");
    ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("{path}/join"),
        json!({}),
    )
    .await?;
    ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("{path}/leave"),
        json!({}),
    )
    .await?;
    let thread = w.thread(&w.users[2], &root).await?;
    assert_eq!(thread["tracked"], true);
    assert_eq!(thread["participant_count"], 0);
    let (status, body) = call(
        &w.app,
        &w.users[2],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::FORBIDDEN, &json!("THREAD_JOIN_REQUIRED"))
    );
    w.fixture.cleanup().await
}

/// 無効化したユーザーは、参加していても、一覧と人数に載らない。
#[tokio::test]
async fn a_deactivated_participant_is_not_listed() -> anyhow::Result<()> {
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
    assert_eq!(w.thread(&w.users[0], &root).await?["participant_count"], 2);
    toasty::sql::statement("UPDATE users SET is_deactivated=true WHERE id=$1")
        .bind(w.users[1].as_str())
        .exec(&mut w.db)
        .await?;
    let thread = w.thread(&w.users[0], &root).await?;
    assert_eq!(thread["participant_count"], 1);
    assert!(
        !thread["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == w.users[1].as_str())
    );
    w.fixture.cleanup().await
}

/// 段階移行の途中（`collecting`・`normalizing`）でも、ゲートは同じに働く。
#[tokio::test]
async fn the_gate_holds_while_the_staged_migration_is_running() -> anyhow::Result<()> {
    for phase in ["collecting", "normalizing"] {
        let w = world_in(phase).await?;
        let root = w.root().await?;
        ok(
            &w.app,
            &w.users[1],
            "POST",
            "/api/messages",
            w.reply(&root, "first"),
        )
        .await?;
        let (status, body) = call(
            &w.app,
            &w.users[2],
            "POST",
            "/api/messages",
            w.reply(&root, "late"),
        )
        .await?;
        assert_eq!(status, StatusCode::FORBIDDEN, "{phase}: {body}");
        assert_eq!(body["code"], "THREAD_JOIN_REQUIRED", "{phase}");
        ok(
            &w.app,
            &w.users[2],
            "POST",
            &format!("/api/threads/{root}/join"),
            json!({}),
        )
        .await?;
        let (status, body) = call(
            &w.app,
            &w.users[2],
            "POST",
            "/api/messages",
            w.reply(&root, "late"),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{phase}: {body}");
        w.fixture.cleanup().await?;
    }
    Ok(())
}

/// ゲスト（チャンネルのメンバーだけを見られる利用者）も、ほかのメンバーと同じにゲートを受ける。
#[tokio::test]
async fn a_guest_is_gated_like_any_other_member() -> anyhow::Result<()> {
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
    toasty::sql::statement("UPDATE users SET is_guest=true WHERE id=$1")
        .bind(w.users[2].as_str())
        .exec(&mut w.db)
        .await?;
    let (status, body) = call(
        &w.app,
        &w.users[2],
        "POST",
        "/api/messages",
        w.reply(&root, "late"),
    )
    .await?;
    assert_eq!(
        (status, &body["code"]),
        (StatusCode::FORBIDDEN, &json!("THREAD_JOIN_REQUIRED")),
        "{body}"
    );
    ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("/api/threads/{root}/join"),
        json!({}),
    )
    .await?;
    let (status, body) = call(
        &w.app,
        &w.users[2],
        "POST",
        "/api/messages",
        w.reply(&root, "late"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    w.fixture.cleanup().await
}

/// 参加・退出・再Openの応答の参加者一覧は、コミット後に別の読み取りで取る。その読み取りが時間の上限
/// （`thread_participants_read_timeout_ms`）を超えたら、一覧を省いて応答する。書き込みはコミット済みで、
/// 応答は書き込みの結果を返す。読み取り（GET）には、この上限はかからない。
#[tokio::test]
async fn a_slow_participants_read_after_a_write_is_left_out() -> anyhow::Result<()> {
    let w = world_with("ready", |config| {
        config.thread_participants_read_timeout_ms = 1
    })
    .await?;
    let root = w.root().await?;
    w.crowd(&root, 3_000).await?;
    let joined = ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("/api/threads/{root}/join"),
        json!({}),
    )
    .await?;
    assert_eq!(joined["data"]["membership"], "joined", "{joined}");
    assert!(joined["data"].get("participants").is_none(), "{joined}");
    assert!(
        joined["data"].get("participant_count").is_none(),
        "{joined}"
    );
    // `tracked` is read in the write's own transaction, so a lost list does not lose it.
    assert_eq!(joined["data"]["tracked"], true, "{joined}");
    // The join took effect, and a read with no deadline lists everyone.
    let after = w.thread(&w.users[1], &root).await?;
    assert_eq!(after["membership"], "joined");
    assert_eq!(after["participant_count"], 3_001);
    // The helper itself: Postgres cancels the statement at the deadline, and without a deadline
    // as short as that, the same read succeeds.
    let viewer = AuthUser {
        id: w.users[1].clone(),
        user_name: w.users[1].clone(),
        display_name: w.users[1].clone(),
    };
    let mut db = w.db.clone();
    assert!(
        read_participants(&mut db, &viewer, &root, &w.channel, 1)
            .await
            .is_err()
    );
    let participants = read_participants(&mut db, &viewer, &root, &w.channel, 5_000).await?;
    assert_eq!(participants["count"], 3_001);
    w.fixture.cleanup().await
}

async fn read(
    db: &mut toasty::Db,
    viewer: &AuthUser,
    root: &str,
    channel: &str,
) -> Result<Value, backend::error::AppError> {
    read_participants(db, viewer, root, channel, 5_000).await
}

/// 書き込みのコミット後の読み取りは、閲覧者がチャンネルを読めるかを、もう一度確かめる。コミットと読み取りの
/// あいだに権限が外れても、プロフィールを返さない。読める条件は GET と同じ（メンバー、または公開チャンネルの
/// ゲスト以外）。
#[tokio::test]
async fn the_list_after_a_write_needs_the_viewer_to_still_have_access() -> anyhow::Result<()> {
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
    let viewer = AuthUser {
        id: w.users[1].clone(),
        user_name: w.users[1].clone(),
        display_name: w.users[1].clone(),
    };
    let mut db = w.db.clone();
    // A member: the parent author and the replier.
    assert_eq!(read(&mut db, &viewer, &root, &w.channel).await?["count"], 2);
    // A public channel can be read without a membership, as `GET` allows.
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2")
        .bind(w.channel.as_str())
        .bind(viewer.id.as_str())
        .exec(&mut db)
        .await?;
    // The viewer is no longer listed itself (a participant who left the channel is not), but the read works.
    assert_eq!(read(&mut db, &viewer, &root, &w.channel).await?["count"], 1);
    // A guest has no access to a channel it is not a member of: nothing is listed.
    toasty::sql::statement("UPDATE users SET is_guest=true WHERE id=$1")
        .bind(viewer.id.as_str())
        .exec(&mut db)
        .await?;
    assert!(read(&mut db, &viewer, &root, &w.channel).await.is_err());
    w.fixture.cleanup().await
}

/// 公開チャンネルは、メンバーでなくても読める。その人の `leave` は、すでにある参加行を終えるだけで、
/// 新しい行は作らない。行が1つでもあるスレッドは tracked になり、メンバー全員の最初の返信に参加が要る
/// ようになるため（メンバーの `leave` は、行がなくても退出の記録を残す）。
#[tokio::test]
async fn a_reader_who_is_not_a_member_cannot_make_a_thread_tracked_by_leaving() -> anyhow::Result<()>
{
    let w = world().await?;
    let root = w.root().await?;
    let outsider = uuid::Uuid::new_v4().to_string();
    ok(
        &w.app,
        &outsider,
        "POST",
        "/api/users",
        json!({"id":outsider,"user_name":outsider,"display_name":outsider}),
    )
    .await?;
    let path = format!("/api/threads/{root}");
    let left = ok(
        &w.app,
        &outsider,
        "POST",
        &format!("{path}/leave"),
        json!({}),
    )
    .await?;
    assert_eq!(left["data"]["membership"], "none", "{left}");
    assert_eq!(left["data"]["tracked"], false, "{left}");
    // Nothing was written: the thread is still untracked, and a member's first reply needs no join.
    assert_eq!(w.thread(&w.users[1], &root).await?["tracked"], false);
    let (status, body) = call(
        &w.app,
        &w.users[1],
        "POST",
        "/api/messages",
        w.reply(&root, "first"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A member who is not in the thread yet may still leave it: the row records the leave.
    let second = w.root().await?;
    let left = ok(
        &w.app,
        &w.users[2],
        "POST",
        &format!("/api/threads/{second}/leave"),
        json!({}),
    )
    .await?;
    assert_eq!(left["data"]["membership"], "left", "{left}");
    assert_eq!(left["data"]["tracked"], true, "{left}");
    // A reader who left the channel keeps the right to end the row it has.
    ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("/api/threads/{second}/join"),
        json!({}),
    )
    .await?;
    let mut db = w.db.clone();
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2")
        .bind(w.channel.as_str())
        .bind(w.users[1].as_str())
        .exec(&mut db)
        .await?;
    let left = ok(
        &w.app,
        &w.users[1],
        "POST",
        &format!("/api/threads/{second}/leave"),
        json!({}),
    )
    .await?;
    assert_eq!(left["data"]["membership"], "left", "{left}");
    w.fixture.cleanup().await
}
