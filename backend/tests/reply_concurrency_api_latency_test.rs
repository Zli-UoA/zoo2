//! Measurement only: `POST /api/messages` replies, 8 clients at a time, to one thread root whose
//! channel has exactly 10,000 `all` notification recipients. All eight writers are joined thread
//! participants, so without `mute` every reply would add the other seven writers as `reply`
//! targets (T=10,007). They are set to `mute` before the measured rounds: each reply then notifies
//! exactly 10,000 users (1 `reply` to the parent author + 9,999 `channel_message`). The row count
//! per reply is verified with SQL after every round, outside the measured interval, before the
//! notification tables are truncated. A reply rejected with 503 while waiting for the root-row lock
//! is timed and counted separately: it must leave no post or notification (checked with SQL), and
//! it contributes no stage samples. Only committed replies feed the percentiles and the breakdown.
//!
//! Times the whole in-process request: router, extractors, dev auth, pool wait, all SQL round trips
//! and the JSON response body. Reports p50 / p95 / max. HTTP network is excluded because the
//! router runs in-process. The write path also emits `notify_timing` events at `debug!` level; a
//! layer installed for this test enables that target at debug and collects the events per reply
//! (keyed by message ID and stage) so connection acquisition, root-row lock, each notification SQL
//! call and COMMIT can be reported separately.
//! Each per-SQL figure is a round trip including Postgres planning and execution: toasty exposes no
//! hook that splits those two, so only a whole-`EXPLAIN` could separate `Planning Time`. A probe
//! acquires a connection from the same pool under the same load as an independent reference. The
//! SQL-layer numbers in `docs/query-design.md` (lock wait, INSERT + COMMIT) are a subset of this
//! request. The build is unoptimized and the database is local, so these are not production numbers.
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
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Barrier;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing_subscriber::{Layer, filter::Targets, layer::Context, prelude::*};

const WRITERS: usize = 8;
const RECIPIENTS: i32 = 10_000;

/// A reply either committed or was rejected with 503 while waiting for the thread-root lock
/// (`lock_timeout`). Both are timed, but only committed replies have notifications and stage
/// events to aggregate.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplyKind {
    Ok,
    ServiceUnavailable,
}

/// One reply's timed outcome, reported to the coordinator before the round's end barrier so the
/// per-round SQL check can tell committed replies from rejected ones.
struct ReplyOutcome {
    round: usize,
    index: usize,
    kind: ReplyKind,
    elapsed_ms: f64,
}

/// `notify_timing` events from the reply write path, keyed by (message ID, stage) so a stage is
/// recorded at most once per reply. Duplicates are kept separately to fail the measurement.
#[derive(Default)]
struct TimingEvents {
    samples: HashMap<(String, String), u64>,
    duplicates: Vec<(String, String)>,
}

#[derive(Default)]
struct TimingVisitor {
    message_id: Option<String>,
    stage: Option<String>,
    elapsed_us: Option<u64>,
}

impl Visit for TimingVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message_id" => self.message_id = Some(value.to_owned()),
            "stage" => self.stage = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "elapsed_us" {
            self.elapsed_us = Some(value);
        }
    }

    /// `%`-formatted fields are recorded through `Debug` (e.g. IDs); keep them readable.
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        match field.name() {
            "message_id" => self.message_id = Some(rendered),
            "stage" => self.stage = Some(rendered),
            _ => {}
        }
    }
}

#[derive(Clone, Default)]
struct TimingCollector(Arc<Mutex<TimingEvents>>);

impl<S> Layer<S> for TimingCollector
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify_timing" {
            return;
        }
        let mut visitor = TimingVisitor::default();
        event.record(&mut visitor);
        if let (Some(message_id), Some(stage), Some(elapsed_us)) =
            (visitor.message_id, visitor.stage, visitor.elapsed_us)
        {
            let mut events = self.0.lock().unwrap();
            let key = (message_id, stage);
            if events.samples.insert(key.clone(), elapsed_us).is_some() {
                events.duplicates.push(key);
            }
        }
    }
}

const TIMING_STAGES: [&str; 9] = [
    "conn_acquire",
    "root_lock",
    "notif_channel",
    "notif_members",
    "notif_levels",
    "notif_participants",
    "notif_excluded",
    "notif_insert",
    "commit",
];

/// `emit_timing` is `debug!`, so the layer must explicitly enable that target at debug.
fn timing_subscriber(collector: TimingCollector) -> impl tracing::Subscriber {
    let target = Targets::new().with_target("notify_timing", tracing::Level::DEBUG);
    tracing_subscriber::registry().with(collector.with_filter(target))
}

#[test]
fn timing_events_enable_debug_and_record_duplicate_reply_stages() {
    let collector = TimingCollector::default();
    tracing::subscriber::with_default(timing_subscriber(collector.clone()), || {
        tracing::debug!(target: "notify_timing", stage = "commit", message_id = "reply-1", elapsed_us = 10u64);
        tracing::debug!(target: "notify_timing", stage = "root_lock", message_id = "reply-1", elapsed_us = 30u64);
        tracing::debug!(target: "notify_timing", stage = "commit", message_id = "reply-1", elapsed_us = 20u64);
    });
    let events = collector.0.lock().unwrap();
    assert_eq!(events.samples.len(), 2);
    assert_eq!(
        events.duplicates,
        vec![("reply-1".to_owned(), "commit".to_owned())]
    );
}

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

/// As `summary`, but an empty set (all replies 503 in a short smoke run) is reported as `null`
/// instead of failing the measurement.
fn summary_opt(label: &str, samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        eprintln!("{label}: no samples");
        return Value::Null;
    }
    summary(label, samples)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "inserts 10k recipients and up to 240 replies; measurement only"]
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
    let app = create_router(AppState::new(db.clone(), config.clone()));
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
    // All eight writers are live participants. Without `mute` a reply would add the other seven
    // writers as `reply` targets on top of the 10,000 `all` members (T=10,007). Mute them first so
    // the measured fan-out is exactly 10,000, then confirm the settings were actually stored.
    for writer in &writers {
        ok(
            &app,
            writer,
            "PATCH",
            &format!("/api/channels/{channel}/notification-settings"),
            json!({"level": "mute"}),
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
    let muted: i64 = admin
        .query_one(
            "SELECT count(*)::bigint FROM user_channel_notification_settings \
             WHERE channel_id=$1 AND notification_level='mute' AND user_id=ANY($2::text[])",
            &[&channel, &writers],
        )
        .await?
        .get(0);
    assert_eq!(muted, i64::try_from(WRITERS)?);
    let prepare_ms = prepared.elapsed().as_secs_f64() * 1000.;
    eprintln!("T={targets} prepare_ms={prepare_ms:.1} rounds={rounds}");

    // Replies run on tokio worker threads, so the collector has to be the global default: a
    // thread-local `with_default` on the test thread would miss events from the spawned tasks.
    // Installed after the fixture is ready, so only the measured replies are collected.
    let timing = TimingCollector::default();
    let subscriber = timing_subscriber(timing.clone());
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| anyhow::anyhow!("set_global_default: {error}"))?;

    // A barrier per round keeps exactly WRITERS replies and one pool-acquire probe in flight;
    // truncate only after everyone has answered, as the SQL-layer measurement did.
    let starts: Vec<_> = (0..rounds)
        .map(|_| Arc::new(Barrier::new(WRITERS + 2)))
        .collect();
    let ends: Vec<_> = (0..rounds)
        .map(|_| Arc::new(Barrier::new(WRITERS + 2)))
        .collect();
    // Message IDs are generated before the timed section so the SQL check after each round can
    // count the notifications produced by each individual reply.
    let reply_ids: Arc<Vec<Vec<String>>> = Arc::new(
        (0..rounds)
            .map(|_| {
                (0..WRITERS)
                    .map(|_| uuid::Uuid::new_v4().to_string())
                    .collect()
            })
            .collect(),
    );
    // Every reply reports its outcome before the end barrier. A 503 is an expected busy-thread
    // response, not a measurement error; other statuses and transport failures are.
    let (result_tx, mut result_rx) =
        tokio::sync::mpsc::unbounded_channel::<anyhow::Result<ReplyOutcome>>();
    let mut clients = Vec::new();
    for (index, writer) in writers.iter().enumerate() {
        let (app, writer, root, channel) =
            (app.clone(), writer.clone(), root.clone(), channel.clone());
        let (reply_ids, starts, ends, result_tx) = (
            reply_ids.clone(),
            starts.clone(),
            ends.clone(),
            result_tx.clone(),
        );
        clients.push(tokio::spawn(async move {
            for round in 0..rounds {
                tokio::time::timeout(Duration::from_secs(600), starts[round].wait())
                    .await
                    .map_err(|_| anyhow::anyhow!("start barrier timed out"))?;
                let outcome = async {
                    let body = json!({
                        "id": reply_ids[round][index].clone(),
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
                    let kind = match status {
                        StatusCode::OK => ReplyKind::Ok,
                        StatusCode::SERVICE_UNAVAILABLE => {
                            eprintln!("reply {round}/{index}: 503 (thread busy): {value}");
                            ReplyKind::ServiceUnavailable
                        }
                        other => anyhow::bail!("reply {round}/{index}: unexpected {other} {value}"),
                    };
                    anyhow::Ok(ReplyOutcome {
                        round,
                        index,
                        kind,
                        elapsed_ms: elapsed,
                    })
                }
                .await;
                let _ = result_tx
                    .send(outcome.map_err(|error| error.context(format!("reply {round}/{index}"))));
                // Even a rejected reply must reach the barrier, or the other clients wait forever.
                tokio::time::timeout(Duration::from_secs(600), ends[round].wait())
                    .await
                    .map_err(|_| anyhow::anyhow!("end barrier timed out"))?;
            }
            anyhow::Ok(())
        }));
    }
    // Same-pool probe: one acquisition per round under the same eight concurrent replies. This is
    // not the router's own acquisition (toasty owns the pool inside AppState), so it is only a
    // reference value for pool wait; it cannot be attributed to a particular request.
    let probe_db = db.clone();
    let (probe_starts, probe_ends, probe_result_tx) =
        (starts.clone(), ends.clone(), result_tx.clone());
    let probe = tokio::spawn(async move {
        let mut samples = Vec::new();
        for round in 0..rounds {
            tokio::time::timeout(Duration::from_secs(600), probe_starts[round].wait())
                .await
                .map_err(|_| anyhow::anyhow!("probe start barrier timed out"))?;
            let began = Instant::now();
            match probe_db.connection().await {
                Ok(connection) => {
                    samples.push(began.elapsed().as_secs_f64() * 1000.);
                    drop(connection);
                }
                Err(error) => {
                    let _ =
                        probe_result_tx.send(Err(anyhow::anyhow!("pool probe {round}: {error:#}")));
                }
            }
            tokio::time::timeout(Duration::from_secs(600), probe_ends[round].wait())
                .await
                .map_err(|_| anyhow::anyhow!("probe end barrier timed out"))?;
        }
        anyhow::Ok(samples)
    });
    drop(result_tx);
    let mut outcomes: Vec<ReplyOutcome> = Vec::new();
    for round in 0..rounds {
        tokio::time::timeout(Duration::from_secs(600), starts[round].wait())
            .await
            .map_err(|_| anyhow::anyhow!("start barrier timed out"))?;
        tokio::time::timeout(Duration::from_secs(600), ends[round].wait())
            .await
            .map_err(|_| anyhow::anyhow!("end barrier timed out"))?;
        // Clients and the probe report before the end barrier, so every outcome for this round is
        // already queued. A transport/setup error is a measurement failure, not a busy reply.
        while let Ok(result) = result_rx.try_recv() {
            match result {
                Ok(outcome) => outcomes.push(outcome),
                Err(error) => anyhow::bail!("{error}"),
            }
        }
        let round_outcomes: Vec<&ReplyOutcome> = outcomes
            .iter()
            .filter(|outcome| outcome.round == round)
            .collect();
        anyhow::ensure!(
            round_outcomes.len() == WRITERS,
            "round {round}: received {} of {WRITERS} reply outcomes",
            round_outcomes.len()
        );
        let ok_ids: Vec<String> = round_outcomes
            .iter()
            .filter(|outcome| outcome.kind == ReplyKind::Ok)
            .map(|outcome| reply_ids[round][outcome.index].clone())
            .collect();
        let busy_ids: Vec<String> = round_outcomes
            .iter()
            .filter(|outcome| outcome.kind == ReplyKind::ServiceUnavailable)
            .map(|outcome| reply_ids[round][outcome.index].clone())
            .collect();
        eprintln!(
            "round {round}: success={} service_unavailable={}",
            ok_ids.len(),
            busy_ids.len()
        );
        // A 503 is returned on the root-row lock timeout, before any INSERT, and the transaction
        // rolls back: neither a message nor a notification may be left behind.
        for message_id in &busy_ids {
            let row = admin
                .query_one(
                    "SELECT (SELECT count(*)::bigint FROM messages WHERE id=$1), \
                            (SELECT count(*)::bigint FROM channel_notifications WHERE message_id=$1)",
                    &[&message_id],
                )
                .await?;
            let posts: i64 = row.get(0);
            let notifications: i64 = row.get(1);
            anyhow::ensure!(
                posts == 0 && notifications == 0,
                "round {round} reply {message_id} returned 503 but left posts={posts} notifications={notifications}"
            );
        }
        // Outside the measured interval: every committed reply must have produced exactly 10,000
        // rows — one `reply` to the parent author and 9,999 `channel_message` to the `all` members.
        // If a writer had been left as a live participant it would be 10,007 instead.
        if !ok_ids.is_empty() {
            let counts = admin
                .query(
                    "SELECT message_id, count(*)::bigint, \
                            count(*) FILTER (WHERE notification_type='reply')::bigint, \
                            count(*) FILTER (WHERE notification_type='channel_message')::bigint \
                     FROM channel_notifications WHERE message_id = ANY($1::text[]) \
                     GROUP BY message_id",
                    &[&ok_ids],
                )
                .await?;
            anyhow::ensure!(
                counts.len() == ok_ids.len(),
                "round {round}: found {} of {} committed replies",
                counts.len(),
                ok_ids.len()
            );
            for row in &counts {
                let message_id: &str = row.get(0);
                let total: i64 = row.get(1);
                let replies: i64 = row.get(2);
                let messages: i64 = row.get(3);
                anyhow::ensure!(
                    total == i64::from(RECIPIENTS)
                        && replies == 1
                        && messages == i64::from(RECIPIENTS) - 1,
                    "round {round} reply {message_id}: total={total} reply={replies} channel_message={messages}"
                );
            }
        }
        admin
            .batch_execute(
                "TRUNCATE notification_events, notifications, apub_notifications, display_refresh_queue",
            )
            .await?;
    }
    for client in clients {
        client.await??;
    }
    let pool_samples = probe.await??;
    let ok_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == ReplyKind::Ok)
        .count();
    let busy_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == ReplyKind::ServiceUnavailable)
        .count();
    anyhow::ensure!(
        ok_count + busy_count == rounds * WRITERS,
        "timed {} replies, expected {}",
        ok_count + busy_count,
        rounds * WRITERS
    );
    let ok_samples: Vec<f64> = outcomes
        .iter()
        .filter(|outcome| outcome.kind == ReplyKind::Ok)
        .map(|outcome| outcome.elapsed_ms)
        .collect();
    let busy_samples: Vec<f64> = outcomes
        .iter()
        .filter(|outcome| outcome.kind == ReplyKind::ServiceUnavailable)
        .map(|outcome| outcome.elapsed_ms)
        .collect();
    eprintln!("replies: success={ok_count} service_unavailable={busy_count}");
    let mut report = serde_json::Map::new();
    report.insert("prepare_ms".into(), json!(prepare_ms));
    report.insert("t".into(), json!(targets));
    report.insert(
        "notifications_per_reply".into(),
        json!(i64::from(RECIPIENTS)),
    );
    report.insert("success_count".into(), json!(ok_count));
    report.insert("service_unavailable_count".into(), json!(busy_count));
    // `api_total` and `api_stage_ms` stay tied to the same committed replies. A 503 has no fan-out
    // to break down, so only its response time is summarized separately.
    report.insert(
        "api_total".into(),
        summary_opt("POST reply API total — success (8 concurrent)", ok_samples),
    );
    report.insert(
        "api_total_service_unavailable".into(),
        summary_opt("POST reply API total — 503 (8 concurrent)", busy_samples),
    );
    report.insert(
        "pool_acquire_probe".into(),
        summary("Pool acquire probe (same pool, 8 concurrent)", pool_samples),
    );
    // Events are keyed by (message ID, stage). A duplicate means the same stage was recorded twice
    // for one reply, so fail before aggregating rather than counting it as another sample. Other
    // posts (fixture setup) and 503 replies are filtered out; every committed reply must then have
    // exactly the nine expected stages, one each, or the measurement fails instead of mixing a
    // partial breakdown into the percentiles.
    let events = timing.0.lock().unwrap();
    anyhow::ensure!(
        events.duplicates.is_empty(),
        "duplicate notify_timing events: {:?}",
        events.duplicates
    );
    let ok_ids: HashSet<&str> = outcomes
        .iter()
        .filter(|outcome| outcome.kind == ReplyKind::Ok)
        .map(|outcome| reply_ids[outcome.round][outcome.index].as_str())
        .collect();
    let mut stage_samples: HashMap<&'static str, Vec<f64>> = TIMING_STAGES
        .iter()
        .map(|stage| (*stage, Vec::new()))
        .collect();
    let mut stages_per_reply: HashMap<String, HashSet<&'static str>> = HashMap::new();
    for ((message_id, stage), elapsed_us) in &events.samples {
        if !ok_ids.contains(message_id.as_str()) {
            continue;
        }
        let Some(stage) = TIMING_STAGES
            .iter()
            .find(|candidate| **candidate == stage.as_str())
        else {
            continue;
        };
        stage_samples
            .get_mut(*stage)
            .unwrap()
            .push(*elapsed_us as f64 / 1000.);
        stages_per_reply
            .entry(message_id.clone())
            .or_default()
            .insert(*stage);
    }
    for message_id in &ok_ids {
        let stages = stages_per_reply.get(*message_id);
        anyhow::ensure!(
            stages.is_some_and(|stages| {
                stages.len() == TIMING_STAGES.len()
                    && TIMING_STAGES.iter().all(|stage| stages.contains(stage))
            }),
            "reply {message_id}: expected all {} stages exactly once, got {:?}",
            TIMING_STAGES.len(),
            stages
        );
    }
    anyhow::ensure!(
        stages_per_reply.len() == ok_count,
        "timings for {} committed replies, expected {}",
        stages_per_reply.len(),
        ok_count
    );
    if ok_count > 0 {
        let mut breakdown = serde_json::Map::new();
        for stage in TIMING_STAGES {
            breakdown.insert(
                stage.into(),
                summary(
                    &format!("{stage} (round trip incl. execution, 8 concurrent)"),
                    stage_samples.remove(stage).unwrap(),
                ),
            );
        }
        report.insert("api_stage_ms".into(), Value::Object(breakdown));
    } else {
        eprintln!("no committed replies; skipping api_stage_ms");
        report.insert("api_stage_ms".into(), Value::Null);
    }
    eprintln!("REPLY_LATENCY_JSON {}", Value::Object(report));
    fixture.cleanup().await?;
    Ok(())
}
