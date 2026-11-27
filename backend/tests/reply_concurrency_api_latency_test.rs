//! Measurement only: `POST /api/messages` replies, 8 clients at a time, to one thread root whose
//! channel has ~10,000 `all` notification recipients. Times the whole in-process request: router,
//! extractors, dev auth, pool wait, all SQL round trips and the JSON response body. It reports
//! p50 / p95 / max; it asserts only that every reply succeeds. The SQL-layer numbers in
//! `docs/query-design.md` (lock wait, INSERT + COMMIT) are a subset of this request: they do not
//! include the member/setting reads, the message writes or the response. The build is unoptimized
//! and the database is local, so these are not production numbers.
//!
//! Run (a database whose role may `SET session_replication_role`; the test makes its own schema):
//! `DATABASE_URL=postgres://… ACTIVITYPUB_BASE_URL=https://zoo.test cargo test --manifest-path
//! backend/Cargo.toml --test reply_concurrency_api_latency_test -- --ignored --nocapture`.
//! CI does not run it. One round holds the root row for the whole fan-out, so 30 rounds take
//! minutes. `REPLY_LATENCY_ROUNDS` lowers the round count for a smoke run.
#[path = "support/thread_db.rs"]
mod thread_db;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Barrier;
use tower::ServiceExt;

const WRITERS: usize = 8;
const RECIPIENTS: i32 = 10_000;

async fn call(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", user)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn ok(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<Value> {
    let (status, value) = call(app, user, method, path, body).await?;
    anyhow::ensure!(status.is_success(), "{method} {path}: {status} {value}");
    Ok(value)
}

/// Nearest-rank percentile, as in the SQL-layer measurement (`ceil(p × n)`-th sample).
fn summary(label: &str, mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    assert!(n > 0);
    let at = |q: f64| samples[((n as f64 * q).ceil() as usize).clamp(1, n) - 1];
    let value = json!({"n":n,"p50":at(0.5),"p95":at(0.95),"max":samples[n - 1]});
    eprintln!("{label}: {value}");
    value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "inserts 10k recipients and 240 replies; measurement only"]
async fn eight_concurrent_replies_api_latency() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let rounds: usize = std::env::var("REPLY_LATENCY_ROUNDS")
        .map(|value| value.parse())
        .unwrap_or(Ok(30))?;
    let rounds = rounds.max(1);
    let prepared = Instant::now();
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let owner = uuid::Uuid::new_v4().to_string();
    let writers: Vec<String> = (0..WRITERS)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect();
    for user in std::iter::once(&owner).chain(writers.iter()) {
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
    for writer in &writers {
        ok(
            &app,
            writer,
            "POST",
            &format!("/api/channels/{channel}/join"),
            json!({}),
        )
        .await?;
    }
    let root = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/messages",
        json!({"id":root,"channel_id":channel,"content":"Root"}),
    )
    .await?;
    // A committed reply creates the thread's participant rows, after which every writer must be a
    // joined participant to reply. Join them before the measured rounds.
    for writer in &writers {
        ok(
            &app,
            writer,
            "POST",
            &format!("/api/threads/{root}/join"),
            json!({}),
        )
        .await?;
    }

    // The channel's `all` recipients: one `users` row, one membership and one settings row each.
    // Replica role skips the per-row app/display triggers of the fixture; the notification INSERT
    // under measurement runs with all production triggers enabled.
    let (mut admin, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let tx = admin.transaction().await?;
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    let recipient_id = "(md5($1||'-r-'||n::text)::uuid)::text";
    tx.execute(
        &format!("INSERT INTO users(id,user_name,display_name) SELECT {recipient_id},{recipient_id},'R' FROM generate_series(1,$2::integer)n"),
        &[&tag, &(RECIPIENTS - 1)],
    )
    .await?;
    tx.execute(
        &format!("INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT {recipient_id},$2,now() FROM generate_series(1,$3::integer)n"),
        &[&tag, &channel, &(RECIPIENTS - 1)],
    )
    .await?;
    tx.execute(
        &format!("INSERT INTO user_channel_notification_settings(user_id,channel_id,notification_level) SELECT {recipient_id},$2,'all' FROM generate_series(1,$3::integer)n"),
        &[&tag, &channel, &(RECIPIENTS - 1)],
    )
    .await?;
    // The parent author is in `all` too, so the 10,000 recipients include the `reply` target.
    tx.execute(
        "UPDATE user_channel_notification_settings SET notification_level='all' WHERE user_id=$1 AND channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    tx.commit().await?;
    admin
        .batch_execute(
            "ANALYZE users; ANALYZE channel_user; ANALYZE user_channel_notification_settings; ANALYZE messages",
        )
        .await?;
    let targets: i64 = admin
        .query_one(
            "SELECT count(*)::bigint FROM user_channel_notification_settings WHERE channel_id=$1 AND notification_level='all'",
            &[&channel],
        )
        .await?
        .get(0);
    assert_eq!(targets, i64::from(RECIPIENTS));
    let prepare_ms = prepared.elapsed().as_secs_f64() * 1000.;
    eprintln!("T={targets} prepare_ms={prepare_ms:.1} rounds={rounds}");

    // A barrier per round keeps exactly WRITERS replies in flight; truncate only after everyone
    // has answered, as the SQL-layer measurement did.
    let starts: Vec<_> = (0..rounds)
        .map(|_| Arc::new(Barrier::new(WRITERS + 1)))
        .collect();
    let ends: Vec<_> = (0..rounds)
        .map(|_| Arc::new(Barrier::new(WRITERS + 1)))
        .collect();
    let (error_tx, mut error_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut clients = Vec::new();
    for (index, writer) in writers.iter().enumerate() {
        let (app, writer, root, channel) =
            (app.clone(), writer.clone(), root.clone(), channel.clone());
        let (starts, ends, error_tx) = (starts.clone(), ends.clone(), error_tx.clone());
        clients.push(tokio::spawn(async move {
            let mut samples = Vec::new();
            for round in 0..rounds {
                tokio::time::timeout(Duration::from_secs(600), starts[round].wait())
                    .await
                    .map_err(|_| anyhow::anyhow!("start barrier timed out"))?;
                let outcome = async {
                    let body = json!({
                        "id": uuid::Uuid::new_v4().to_string(),
                        "channel_id": channel,
                        "parent_id": root,
                        "content": format!("reply {round} from {index}"),
                    });
                    let request = Request::builder()
                        .method("POST")
                        .uri("/api/messages")
                        .header("X-Dev-User", &writer)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))?;
                    let began = Instant::now();
                    let response = app.clone().oneshot(request).await?;
                    let status = response.status();
                    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
                    let elapsed = began.elapsed().as_secs_f64() * 1000.;
                    let value: Value = serde_json::from_slice(&bytes)?;
                    anyhow::ensure!(
                        status.is_success(),
                        "reply {round}/{index}: {status} {value}"
                    );
                    anyhow::Ok(elapsed)
                }
                .await;
                match outcome {
                    Ok(elapsed) => samples.push(elapsed),
                    Err(error) => {
                        let _ = error_tx.send(format!("reply {round}/{index}: {error:#}"));
                    }
                }
                // Even a failed reply must reach the barrier, or the other clients wait forever.
                tokio::time::timeout(Duration::from_secs(600), ends[round].wait())
                    .await
                    .map_err(|_| anyhow::anyhow!("end barrier timed out"))?;
            }
            anyhow::Ok(samples)
        }));
    }
    drop(error_tx);
    for round in 0..rounds {
        tokio::time::timeout(Duration::from_secs(600), starts[round].wait())
            .await
            .map_err(|_| anyhow::anyhow!("start barrier timed out"))?;
        tokio::time::timeout(Duration::from_secs(600), ends[round].wait())
            .await
            .map_err(|_| anyhow::anyhow!("end barrier timed out"))?;
        if let Ok(error) = error_rx.try_recv() {
            anyhow::bail!("{error}");
        }
        admin
            .batch_execute(
                "TRUNCATE notification_events, notifications, apub_notifications, display_refresh_queue",
            )
            .await?;
    }
    let mut samples = Vec::new();
    for client in clients {
        samples.extend(client.await??);
    }
    let mut report = serde_json::Map::new();
    report.insert("prepare_ms".into(), json!(prepare_ms));
    report.insert(
        "api_total".into(),
        summary("POST reply API total (8 concurrent)", samples),
    );
    eprintln!("REPLY_LATENCY_JSON {}", Value::Object(report));
    fixture.cleanup().await?;
    Ok(())
}
