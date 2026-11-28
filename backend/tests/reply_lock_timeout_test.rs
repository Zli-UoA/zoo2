//! 返信のルート行ロック待ちが statement_timeout (5 秒) で打ち切られる経路を、通常の CI テストで
//! 検証する。別セッションがルート行を保持している間の POST は 503 + `retry-after: 5` になり、
//! 投稿も通知も残らない。ロックを解放したあと同じ ID で再送すれば 200 になり、二重投稿しない。
//! 本番コードのタイムアウトは変更せず、テスト側の保持時間だけをここで定義する。
#[path = "support/thread_db.rs"]
mod thread_db;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_postgres::NoTls;
use tower::ServiceExt;

/// 本番の `SET LOCAL statement_timeout = '5s'` より長くルート行を保持する。
const LOCK_HOLD: Duration = Duration::from_secs(6);
/// 失敗したテストが延々と待たないための上限（503 までは 5 秒かかる）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

async fn request(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, HeaderMap, Value)> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", user)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, headers, serde_json::from_slice(&bytes)?))
}

async fn ok(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(HeaderMap, Value)> {
    let (status, headers, value) = request(app, user, method, path, body).await?;
    anyhow::ensure!(status.is_success(), "{method} {path}: {status} {value}");
    Ok((headers, value))
}

async fn count(admin: &tokio_postgres::Client, sql: &str, id: &str) -> anyhow::Result<i64> {
    Ok(admin.query_one(sql, &[&id]).await?.get::<_, i64>(0))
}

#[tokio::test]
async fn a_reply_that_times_out_waiting_for_the_root_lock_returns_503_and_is_retryable()
-> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let url = fixture.config.database_url.clone();
    let config = fixture.config.clone();
    let db = init_db(&url).await?;
    let app = create_router(AppState::new(db, config));

    let (owner, author) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    for user in [&owner, &author] {
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
        &owner,
        "POST",
        "/api/channels",
        json!({"id":channel,"channel_name":channel,"is_private":false}),
    )
    .await?;
    ok(
        &app,
        &author,
        "POST",
        &format!("/api/channels/{channel}/join"),
        json!({}),
    )
    .await?;
    let root = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/messages",
        json!({"id":root,"channel_id":channel,"content":"Root"}),
    )
    .await?;
    // A normal reply establishes the participant rows and gives a baseline for the fan-out: the
    // timed-out reply must not add a message or a notification, and the retry must add exactly the
    // same set of notifications as this one.
    let control_id = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &author,
        "POST",
        "/api/messages",
        json!({"id":control_id,"channel_id":channel,"parent_id":root,"content":"control"}),
    )
    .await?;
    let (admin, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let expected_notifications = count(
        &admin,
        "SELECT count(*)::bigint FROM channel_notifications WHERE message_id=$1",
        &control_id,
    )
    .await?;
    anyhow::ensure!(
        expected_notifications > 0,
        "the control reply produced no notifications"
    );

    let target_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "id": target_id,
        "channel_id": channel,
        "parent_id": root,
        "content": "waits for the locked root",
    });
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (lock_url, lock_root) = (url.clone(), root.clone());
    let locker = tokio::spawn(async move {
        let (mut client, connection) = tokio_postgres::connect(&lock_url, NoTls).await?;
        tokio::spawn(connection);
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT id FROM messages WHERE id=$1 FOR NO KEY UPDATE",
            &[&lock_root],
        )
        .await?;
        let locked_at = Instant::now();
        let _ = locked_tx.send(locked_at);
        // The request finishes at the 5-second statement timeout; keep the row locked for six
        // seconds from acquisition so the timeout, not a release, ends the wait.
        let _ = release_rx.await;
        if let Some(remaining) = LOCK_HOLD.checked_sub(locked_at.elapsed()) {
            tokio::time::sleep(remaining).await;
        }
        tx.commit().await?;
        anyhow::Ok(())
    });
    let locked_at = tokio::time::timeout(REQUEST_TIMEOUT, locked_rx)
        .await?
        .map_err(|_| anyhow::anyhow!("the lock holder did not acquire the root row"))?;
    let reply = tokio::time::timeout(
        REQUEST_TIMEOUT,
        request(&app, &author, "POST", "/api/messages", payload.clone()),
    )
    .await;
    // Always let the holder finish, even when the assertion below fails.
    let _ = release_tx.send(());
    let (status, headers, body) =
        reply.map_err(|_| anyhow::anyhow!("the reply did not return in time"))??;
    locker.await??;
    anyhow::ensure!(
        locked_at.elapsed() >= LOCK_HOLD,
        "the root lock was released before the statement timeout could fire"
    );

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("5")
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*)::bigint FROM messages WHERE id=$1",
            &target_id
        )
        .await?,
        0
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*)::bigint FROM channel_notifications WHERE message_id=$1",
            &target_id
        )
        .await?,
        0
    );

    let (status, _, body) = request(&app, &author, "POST", "/api/messages", payload).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], target_id);
    assert_eq!(
        count(
            &admin,
            "SELECT count(*)::bigint FROM messages WHERE id=$1",
            &target_id
        )
        .await?,
        1
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*)::bigint FROM channel_notifications WHERE message_id=$1",
            &target_id
        )
        .await?,
        expected_notifications
    );
    fixture.cleanup().await
}
