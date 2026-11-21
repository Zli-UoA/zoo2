//! Measurement only: `GET /api/threads/{id}` of a thread with ~10,000 participant rows, through the
//! router (extractors, the pool, the SQL round trips, JSON), alone, concurrently, and while the
//! thread's writers (reply, leave, join) hold its row lock. Reports p50 / p95 / max per operation;
//! it asserts only that every call succeeds and the answers are right. The build is unoptimized and
//! the database is local, so the numbers are an upper bound of the CPU and a lower bound of the
//! network. The SQL work itself is bounded by `thread_participants_scale_test`.
#[path = "support/thread_db.rs"]
mod thread_db;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use std::time::Instant;
use tower::ServiceExt;

const P: i32 = 10_000;
const LEFT: i32 = 500;
const DEACTIVATED: i32 = 500;
const NON_MEMBERS: i32 = 500;
const ROUNDS: usize = 30;
const READERS: usize = 8;

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

async fn get(app: Router, user: String, path: String) -> anyhow::Result<Value> {
    ok(&app, &user, "GET", &path, Value::Null).await
}

/// A reply (it takes the thread row lock and runs the trigger), then a leave and a join (both
/// answer with the participants too, read after the commit), ROUNDS times, as a joined user.
async fn write_rounds(
    app: Router,
    user: String,
    root: String,
    channel: String,
) -> anyhow::Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let (mut replies, mut leaves, mut joins) = (Vec::new(), Vec::new(), Vec::new());
    for round in 0..ROUNDS {
        let body = json!({"id":uuid::Uuid::new_v4().to_string(),"channel_id":channel,"parent_id":root,"content":format!("reply {round}")});
        replies.push(
            timed(ok(&app, &user, "POST", "/api/messages", body))
                .await?
                .0,
        );
        let leave = format!("/api/threads/{root}/leave");
        leaves.push(timed(ok(&app, &user, "POST", &leave, json!({}))).await?.0);
        let join = format!("/api/threads/{root}/join");
        joins.push(timed(ok(&app, &user, "POST", &join, json!({}))).await?.0);
    }
    Ok((replies, leaves, joins))
}

/// Runs `operation` and returns how long it took, in ms.
async fn timed<F: std::future::Future<Output = anyhow::Result<Value>>>(
    operation: F,
) -> anyhow::Result<(f64, Value)> {
    let started = Instant::now();
    let value = operation.await?;
    Ok((started.elapsed().as_secs_f64() * 1000., value))
}

fn summary(label: &str, mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let at = |q: f64| samples[((n as f64 * q).ceil() as usize).clamp(1, n) - 1];
    let value = json!({"n":n,"p50":at(0.5),"p95":at(0.95),"max":samples[n - 1]});
    eprintln!("{label}: {value}");
    value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "inserts 10k participants; measurement only"]
async fn thread_detail_latency_with_many_participants() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config.clone()));
    // users[0] owns the channel and the root; users[1] reads; users[2] writes.
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
    let root = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &users[0],
        "POST",
        "/api/messages",
        json!({"id":root,"channel_id":channel,"content":"Root"}),
    )
    .await?;

    // The thread's participants: the three users above, plus P bulk users like the `large` case of
    // the scale test (LEFT left, DEACTIVATED deactivated, NON_MEMBERS no longer channel members).
    let (mut admin, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let tx = admin.transaction().await?;
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) SELECT $1||'-'||n,$1||'-'||n,'U' FROM generate_series(1,$2::integer)n",
        &[&tag, &P],
    )
    .await?;
    tx.execute(
        "INSERT INTO thread_participants(thread_id,user_id,joined_at,left_at) SELECT $1,$2||'-'||n,timestamp '2025-01-01'+n*interval '1 second',CASE WHEN n<=$4::integer THEN timestamp '2025-02-01' END FROM generate_series(1,$3::integer)n",
        &[&root, &tag, &P, &LEFT],
    )
    .await?;
    tx.execute(
        "INSERT INTO thread_participants(thread_id,user_id,joined_at) SELECT $1,u,now() FROM unnest($2::text[])u ON CONFLICT DO NOTHING",
        &[&root, &users],
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $1||'-'||n,$2,now() FROM generate_series(1,$3::integer)n ON CONFLICT DO NOTHING",
        &[&tag, &channel, &P],
    )
    .await?;
    // The first live ones: deactivated (n in LEFT+1..), the last live ones: out of the channel.
    tx.execute(
        "UPDATE users SET is_deactivated=true WHERE id IN (SELECT user_id FROM thread_participants WHERE thread_id=$1 AND left_at IS NULL AND user_id LIKE $2||'-%' ORDER BY joined_at LIMIT $3::integer)",
        &[&root, &tag, &DEACTIVATED],
    )
    .await?;
    tx.execute(
        "DELETE FROM channel_user WHERE channel_id=$2 AND user_id IN (SELECT user_id FROM thread_participants WHERE thread_id=$1 AND left_at IS NULL AND user_id LIKE $3||'-%' ORDER BY joined_at DESC LIMIT $4::integer)",
        &[&root, &channel, &tag, &NON_MEMBERS],
    )
    .await?;
    tx.commit().await?;
    admin
        .batch_execute(
            "ANALYZE users; ANALYZE thread_participants; ANALYZE channel_user; ANALYZE messages",
        )
        .await?;
    let live = i64::from(P - LEFT - DEACTIVATED - NON_MEMBERS) + 3;

    let path = format!("/api/threads/{root}");
    let none_path = format!("{path}?participants=none");
    let detail = |user: &str| get(app.clone(), user.to_owned(), path.clone());
    let tracked_only = |user: &str| get(app.clone(), user.to_owned(), none_path.clone());
    // Correctness first: the count and the first 50.
    let first = detail(&users[1]).await?;
    assert_eq!(first["data"]["participant_count"], live);
    assert_eq!(first["data"]["participants"].as_array().unwrap().len(), 50);
    assert_eq!(first["data"]["tracked"], true);
    let none = tracked_only(&users[1]).await?;
    assert_eq!(none["data"]["tracked"], true);
    assert!(none["data"].get("participants").is_none());
    for _ in 0..3 {
        detail(&users[1]).await?;
    }

    let mut report = serde_json::Map::new();
    // 1. Alone.
    let mut single = Vec::new();
    let mut single_none = Vec::new();
    for _ in 0..ROUNDS {
        single.push(timed(detail(&users[1])).await?.0);
        single_none.push(timed(tracked_only(&users[1])).await?.0);
    }
    report.insert("get_single".into(), summary("GET alone", single));
    report.insert(
        "get_participants_none_single".into(),
        summary("GET ?participants=none alone", single_none),
    );

    // 2. The writer alone, on a thread with three participants of the same channel (the control:
    //    the cost of the request and of the channel's own fan-out, without the thread's P), then on
    //    the thread of P participants.
    let control = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &users[0],
        "POST",
        "/api/messages",
        json!({"id":control,"channel_id":channel,"content":"Control"}),
    )
    .await?;
    ok(
        &app,
        &users[1],
        "POST",
        "/api/messages",
        json!({"id":uuid::Uuid::new_v4().to_string(),"channel_id":channel,"parent_id":control,"content":"First"}),
    )
    .await?;
    for user in &users[1..] {
        ok(
            &app,
            user,
            "POST",
            &format!("/api/threads/{control}/join"),
            json!({}),
        )
        .await?;
    }
    let (replies, leaves, joins) = write_rounds(
        app.clone(),
        users[2].clone(),
        control.clone(),
        channel.clone(),
    )
    .await?;
    report.insert(
        "reply_alone_control".into(),
        summary("POST reply (alone, 3 participants)", replies),
    );
    report.insert(
        "leave_alone_control".into(),
        summary("POST leave (alone, 3 participants)", leaves),
    );
    report.insert(
        "join_alone_control".into(),
        summary("POST join (alone, 3 participants)", joins),
    );
    let (replies, leaves, joins) =
        write_rounds(app.clone(), users[2].clone(), root.clone(), channel.clone()).await?;
    report.insert(
        "reply_alone".into(),
        summary("POST reply (alone, P participants)", replies),
    );
    report.insert(
        "leave_alone".into(),
        summary("POST leave (alone, P participants)", leaves),
    );
    report.insert(
        "join_alone".into(),
        summary("POST join (alone, P participants)", joins),
    );

    // 3. READERS concurrent readers, without and with a writer on the same thread.
    for with_writer in [false, true] {
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let (app, user, path) = (app.clone(), users[1].clone(), path.clone());
            readers.push(tokio::spawn(async move {
                let mut samples = Vec::new();
                for _ in 0..ROUNDS {
                    samples.push(timed(get(app.clone(), user.clone(), path.clone())).await?.0);
                }
                anyhow::Ok(samples)
            }));
        }
        let writer = with_writer.then(|| {
            tokio::spawn(write_rounds(
                app.clone(),
                users[2].clone(),
                root.clone(),
                channel.clone(),
            ))
        });
        let mut all = Vec::new();
        for reader in readers {
            all.extend(reader.await??);
        }
        let label = if with_writer {
            "get_concurrent8_with_writer"
        } else {
            "get_concurrent8"
        };
        report.insert(
            label.into(),
            summary(
                &format!(
                    "GET x{READERS} concurrent{}",
                    if with_writer { " + writer" } else { "" }
                ),
                all,
            ),
        );
        if let Some(writer) = writer {
            let (replies, leaves, joins) = writer.await??;
            report.insert(
                "reply_during_reads".into(),
                summary("POST reply (during reads)", replies),
            );
            report.insert(
                "leave_during_reads".into(),
                summary("POST leave (during reads)", leaves),
            );
            report.insert(
                "join_during_reads".into(),
                summary("POST join (during reads)", joins),
            );
        }
    }
    eprintln!("API_LATENCY_JSON {}", Value::Object(report));
    fixture.cleanup().await?;
    Ok(())
}
