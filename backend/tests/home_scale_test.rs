//! Opt-in scale regression: uses a migrated disposable PostgreSQL database.
//! SCALE_POSTS=1000000 cargo test --test home_scale_test -- --ignored --nocapture
use backend::{
    config::Config,
    routes::home::{timeline_page_query_for, timeline_page_query_for_request, timeline_query_for},
};
use serde_json::Value;
use tokio_postgres::{NoTls, SimpleQueryMessage, Transaction};

fn bind(mut sql: String, args: &[&str]) -> String {
    for (i, value) in args.iter().enumerate().rev() {
        sql = sql.replace(
            &format!("${}", i + 1),
            &format!("'{}'", value.replace('\'', "''")),
        );
    }
    sql
}
async fn plan(tx: &Transaction<'_>, sql: &str) -> anyhow::Result<Value> {
    let result = tx
        .simple_query(&format!("EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) {sql}"))
        .await?;
    let json = result
        .iter()
        .find_map(|r| match r {
            SimpleQueryMessage::Row(row) => row.get(0),
            _ => None,
        })
        .unwrap();
    Ok(serde_json::from_str::<Value>(json)?[0].clone())
}
fn visit(node: &Value, check: &impl Fn(&Value)) {
    check(node);
    if let Some(children) = node["Plans"].as_array() {
        for child in children {
            visit(child, check);
        }
    }
}

#[tokio::test]
#[ignore = "inserts 300k–1m posts; run against a disposable migrated database"]
async fn timeline_work_is_bounded_by_page_and_followees_not_history() -> anyhow::Result<()> {
    let count: usize = std::env::var("SCALE_POSTS")
        .unwrap_or_else(|_| "300000".into())
        .parse()?;
    assert!(count >= 300000);
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL jit=off; SET LOCAL statement_timeout='300s'")
        .await?;
    let user = uuid::Uuid::new_v4().to_string();
    let empty = uuid::Uuid::new_v4().to_string();
    let prefix = format!("https://{user}.test");
    let local = count / 3;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",
        &[&user, &empty],
    )
    .await?;
    tx.execute("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) SELECT $1||'/actors/'||n,$1,$1,'actor','Actor',$1,'dummy' FROM generate_series(0,19)n", &[&prefix]).await?;
    tx.execute("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) SELECT $1,$2||'/actors/'||n,'outgoing',$2||'/follow/'||n,true FROM generate_series(0,19)n", &[&user,&prefix]).await?;
    tx.batch_execute(&format!("INSERT INTO home_posts(id,user_id,content,created_at,visibility) SELECT '{user}-'||lpad(n::text,9,'0'),'{user}',repeat('content ',80),now()-n*interval '1 second','public' FROM generate_series(1,{local})n")).await?;
    tx.batch_execute(&format!("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) SELECT '{prefix}/posts/'||lpad(n::text,9,'0'),'{prefix}/actors/'||(n%20),repeat('content ',80),'public',now()-n*interval '1 second',now() FROM generate_series(1,{})n",count-local)).await?;
    tx.execute("INSERT INTO apub_remote_recipients(post_id,user_id) SELECT id,$1 FROM apub_remote_posts WHERE starts_with(id,$2||'/posts/')", &[&user,&prefix]).await?;
    tx.batch_execute("ANALYZE home_posts; ANALYZE apub_remote_posts; ANALYZE apub_remote_recipients; ANALYZE apub_follows; ANALYZE apub_actors").await?;
    tx.batch_execute("SET LOCAL statement_timeout='30s'")
        .await?;
    let actor = format!("{prefix}/actors/0");
    let deep = format!("{user}-{:09}", local * 4 / 5);
    for (name, viewer, timeline, author, before, expected) in [
        ("home", &user, "home", "", "", 50),
        ("deep", &user, "home", "", deep.as_str(), 50),
        ("local", &user, "local", "", "", 50),
        ("social", &user, "social", "", "", 50),
        ("author", &user, "social", user.as_str(), "", 50),
        ("remote-author", &user, "social", actor.as_str(), "", 50),
        ("empty", &empty, "home", "", "", 0),
        ("invalid-cursor", &user, "home", "", "missing", 0),
    ] {
        let sql = bind(
            timeline_page_query_for_request(
                timeline,
                false,
                !author.is_empty(),
                !before.is_empty(),
            ),
            &[
                viewer, before, timeline, "false", "50", "", author, "false", "[]",
            ],
        );
        let p = plan(&tx, &sql).await?;
        assert_eq!(p["Plan"]["Actual Rows"], expected, "{name}");
        visit(&p["Plan"], &|n| {
            if matches!(
                n["Relation Name"].as_str(),
                Some("home_posts" | "apub_remote_posts" | "apub_remote_recipients")
            ) {
                let examined = (n["Actual Rows"].as_f64().unwrap_or(0.)
                    + n["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                    * n["Actual Loops"].as_f64().unwrap_or(0.);
                assert!(examined <= 5000., "{name} scans history: {n}");
            }
            assert_eq!(
                n["Temp Written Blocks"].as_u64().unwrap_or(0),
                0,
                "{name} spills to disk"
            );
        });
        eprintln!(
            "{count} {name}: plan={}ms execute={}ms",
            p["Planning Time"], p["Execution Time"]
        );
    }
    for timeline in ["local", "social"] {
        let sql = bind(
            timeline_page_query_for(timeline, true),
            &[&empty, "", timeline, "true", "50", "", "", "false", "[]"],
        );
        let p = plan(&tx, &sql).await?;
        assert_eq!(p["Plan"]["Actual Rows"], 0, "empty guest {timeline}");
        visit(&p["Plan"], &|n| {
            if n["Relation Name"] == "home_posts" {
                assert!(
                    n["Actual Rows"].as_f64().unwrap_or(0.)
                        * n["Actual Loops"].as_f64().unwrap_or(0.)
                        <= 50.,
                    "guest scans inaccessible history: {n}"
                );
            }
        });
        eprintln!(
            "{count} empty guest {timeline}: plan={}ms execute={}ms",
            p["Planning Time"], p["Execution Time"]
        );
    }
    // Receiving a large history must not make a now-unfollowed actor a scan.
    tx.execute(
        "UPDATE apub_follows SET accepted=false WHERE user_id=$1",
        &[&user],
    )
    .await?;
    let p = plan(
        &tx,
        &bind(
            timeline_page_query_for_request("home", false, true, false),
            &[&user, "", "home", "false", "50", "", &actor, "false", "[]"],
        ),
    )
    .await?;
    assert_eq!(p["Plan"]["Actual Rows"], 0);
    visit(&p["Plan"], &|n| {
        if n["Relation Name"] == "apub_remote_recipients" {
            assert!(
                n["Actual Rows"].as_f64().unwrap_or(0.) * n["Actual Loops"].as_f64().unwrap_or(0.)
                    <= 50.,
                "unfollowed recipient history scan: {n}"
            );
        }
    });
    tx.execute(
        "UPDATE apub_follows SET accepted=true WHERE user_id=$1",
        &[&user],
    )
    .await?;
    if std::env::var_os("SCALE_HTTP").is_some() {
        tx.commit().await?;
        let result = benchmark_home_requests(&user, count).await;
        client
            .execute(
                "DELETE FROM apub_remote_posts WHERE starts_with(id,$1||'/posts/')",
                &[&prefix],
            )
            .await?;
        client
            .execute("DELETE FROM users WHERE id IN ($1,$2)", &[&user, &empty])
            .await?;
        client
            .execute(
                "DELETE FROM apub_actors WHERE starts_with(id,$1||'/actors/')",
                &[&prefix],
            )
            .await?;
        return result;
    }
    // A wide conversation must not materialize thousands of payloads. Counts
    // remain exact through audience-scoped counters, without scanning replies.
    let root = format!("{prefix}/posts/000000001");
    tx.execute(
        "UPDATE apub_remote_posts SET parent_id=$1 WHERE actor_id=$2 AND id<>$1",
        &[&root, &actor],
    )
    .await?;
    for thread in ["false", "true"] {
        let sql = bind(
            timeline_query_for(thread == "true", false, true, false, false),
            &[&user, "", "social", "false", "500", &root, "", thread, "[]"],
        );
        let p = plan(&tx, &sql).await?;
        visit(&p["Plan"], &|n| {
            if n["Relation Name"] == "apub_remote_recipients" {
                let read = (n["Actual Rows"].as_f64().unwrap_or(0.)
                    + n["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                    * n["Actual Loops"].as_f64().unwrap_or(0.);
                assert!(
                    read <= 5000.,
                    "reply count scanned recipient history instead of counters: {n}"
                );
            }
            if thread == "true" && n["Subplan Name"] == "CTE page_parent_previews" {
                assert_eq!(
                    n["Actual Rows"], 1,
                    "the shared reply parent is encoded once"
                );
                assert_eq!(
                    n["Actual Loops"], 1,
                    "parent preview must not be rebuilt for every child"
                );
            }
            if n["Subplan Name"] == "CTE base" {
                assert!(
                    n["Actual Rows"].as_u64().unwrap() <= 601,
                    "unbounded conversation payloads: {n}"
                );
            }
        });
        let values: Vec<Value> = tx
            .simple_query(&sql)
            .await?
            .iter()
            .filter_map(|r| match r {
                SimpleQueryMessage::Row(row) => row.get(0),
                _ => None,
            })
            .map(|s| backend::routes::home::timeline_post(serde_json::from_str(s).unwrap()))
            .collect();
        let target = values
            .iter()
            .find(|v| v["id"] == root)
            .expect("requested post survives the cap");
        let replies: i64 = tx
            .query_one(
                "SELECT count(*) FROM apub_remote_posts WHERE parent_id=$1",
                &[&root],
            )
            .await?
            .get(0);
        assert_eq!(target["replies"], replies, "exact visible reply count");
        assert!(values.len() <= 500);
        eprintln!(
            "{count} detail thread={thread}: plan={}ms execute={}ms",
            p["Planning Time"], p["Execution Time"]
        );
    }
    if std::env::var_os("SCALE_BUDGET").is_some() {
        tx.commit().await?;
        verify_budget(&user, &root).await?;
    } else {
        tx.rollback().await?;
    }
    Ok(())
}

// Exercise the complete Axum handler, pool acquisition, decorations and JSON,
// without network noise. Timings are observations, never flaky CI thresholds.
async fn benchmark_home_requests(user: &str, count: usize) -> anyhow::Result<()> {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use backend::{db::init_db, routes::create_router, state::AppState};
    use tower::ServiceExt;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config));
    let mut elapsed = Vec::new();
    for _ in 0..3 {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let app = app.clone();
            let user = user.to_owned();
            tasks.spawn(async move {
                let start = std::time::Instant::now();
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/api/home?timeline=home")
                            .header("X-Dev-User", user)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                    .await
                    .unwrap();
                let rows: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(rows.as_array().unwrap().len(), 50);
                start.elapsed().as_secs_f64() * 1000.
            });
        }
        while let Some(value) = tasks.join_next().await {
            elapsed.push(value?);
        }
    }
    elapsed.sort_by(f64::total_cmp);
    eprintln!(
        "{count} full Home handler, 8 concurrent / 24 requests: p50={:.1}ms p95={:.1}ms max={:.1}ms (debug build)",
        elapsed[12], elapsed[22], elapsed[23]
    );
    Ok(())
}

// A dedicated benchmark gate: every sample includes PostgreSQL planning and
// execution. Keep fixtures committed for independent concurrent connections.
async fn verify_budget(user: &str, root: &str) -> anyhow::Result<()> {
    let url = Config::from_env().database_url;
    let (fixture, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    // Fixture positioning, outside the timed application queries.
    let deep:String=fixture.query_one("SELECT id FROM home_posts WHERE user_id=$1 ORDER BY created_at,id LIMIT 1 OFFSET 10000", &[&user]).await?.get(0);
    let remote_author = format!("{}/actors/0", root.split("/posts/").next().unwrap());
    let reply = root.replace("/posts/000000001", "/posts/000000020");
    let writer = format!("{user}-budget-writer");
    fixture
        .execute(
            "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text) ON CONFLICT DO NOTHING",
            &[&writer],
        )
        .await?;
    let page = bind(
        timeline_page_query_for("home", false),
        &[user, "", "home", "false", "500", "", "", "false", "[]"],
    );
    let entries: Vec<String> = fixture
        .simple_query(&page)
        .await?
        .iter()
        .filter_map(|r| match r {
            SimpleQueryMessage::Row(row) => row.get(0).map(|s| {
                serde_json::from_str::<Value>(s).unwrap()["entry_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }),
            _ => None,
        })
        .collect();
    assert_eq!(entries.len(), 500, "exercise the full supported page size");
    let entries = serde_json::to_string(&entries)?;
    let make = |name: &str, sql: String, args: &[&str]| {
        (
            name.to_owned(),
            sql,
            args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    };
    let queries = [
        make(
            "home",
            timeline_page_query_for("home", false),
            &[user, "", "home", "false", "50", "", "", "false", "[]"],
        ),
        make(
            "home_max",
            timeline_page_query_for("home", false),
            &[user, "", "home", "false", "500", "", "", "false", "[]"],
        ),
        make(
            "page_render",
            timeline_query_for(false, false, false, false, false),
            &[user, "", "home", "false", "500", "", "", "false", &entries],
        ),
        make(
            "deep",
            timeline_page_query_for_request("home", false, false, true),
            &[user, &deep, "home", "false", "50", "", "", "false", "[]"],
        ),
        make(
            "local",
            timeline_page_query_for("local", false),
            &[user, "", "local", "false", "50", "", "", "false", "[]"],
        ),
        make(
            "social",
            timeline_page_query_for("social", false),
            &[user, "", "social", "false", "50", "", "", "false", "[]"],
        ),
        make(
            "author",
            timeline_page_query_for_request("social", false, true, false),
            &[user, "", "social", "false", "50", "", user, "false", "[]"],
        ),
        make(
            "remote_author",
            timeline_page_query_for_request("social", false, true, false),
            &[
                user,
                "",
                "social",
                "false",
                "50",
                "",
                &remote_author,
                "false",
                "[]",
            ],
        ),
        make(
            "guest",
            timeline_page_query_for("social", true),
            &[
                "empty-budget-user",
                "",
                "social",
                "true",
                "50",
                "",
                "",
                "false",
                "[]",
            ],
        ),
        make(
            "detail",
            timeline_query_for(false, false, true, false, false),
            &[user, "", "social", "false", "500", root, "", "false", "[]"],
        ),
        make(
            "thread",
            timeline_query_for(true, false, true, false, false),
            &[user, "", "social", "false", "500", root, "", "true", "[]"],
        ),
        make(
            "receipt_insert",
            "WITH bound_params AS (SELECT $1::text,$2::text,$3::text,$4::text,$5::text,$6::text,$7::text,$8::text,$9::text) INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($6,$1) RETURNING post_id".into(),
            &[&writer, "", "", "", "", &reply, "", "", ""],
        ),
        make(
            "post_delete",
            "WITH bound_params AS (SELECT $1::text,$2::text,$3::text,$4::text,$5::text,$6::text,$7::text,$8::text,$9::text) UPDATE apub_remote_posts SET deleted_at=now() WHERE id=$6 RETURNING id".into(),
            &[user, "", "", "", "", &reply, "", "", ""],
        ),
    ];
    let mut connections = Vec::new();
    // Writes do not use Home's admission limiter; exercise eight DB writers.
    for _ in 0..8 {
        let (client, connection) = tokio_postgres::connect(&url, NoTls).await?;
        tokio::spawn(connection);
        client
            .batch_execute("SET jit=off; SET plan_cache_mode=force_generic_plan")
            .await?;
        connections.push(std::sync::Arc::new(tokio::sync::Mutex::new((
            client,
            std::collections::HashSet::<String>::new(),
        ))));
    }
    let pool = std::sync::Arc::new(connections);
    let mut violations = Vec::new();
    for (name, sql, args) in queries {
        for concurrency in [1, 8] {
            let mut tasks = tokio::task::JoinSet::new();
            let pool = if is_budget_write(&name) {
                pool.clone()
            } else {
                std::sync::Arc::new(
                    pool.iter()
                        .take(backend::routes::home::HOME_QUERY_CONCURRENCY)
                        .cloned()
                        .collect(),
                )
            };
            for worker in 0..concurrency {
                let pool = pool.clone();
                let sql = sql.clone();
                let args = args.clone();
                let name = name.clone();
                tasks.spawn(budget_samples(pool, worker, name, sql, args));
            }
            let mut samples = Vec::new();
            let mut waits = Vec::new();
            let mut round_trips = Vec::new();
            while let Some(result) = tasks.join_next().await {
                let (s, w, r) = result??;
                samples.extend(s);
                waits.extend(w);
                round_trips.extend(r);
            }
            samples.sort_by(f64::total_cmp);
            waits.sort_by(f64::total_cmp);
            round_trips.sort_by(f64::total_cmp);
            eprintln!(
                "budget {name} concurrency={concurrency} samples={} p50={:.2} p95={:.2} max={:.2}; pool-wait p95={:.2} max={:.2}",
                samples.len(),
                samples[samples.len() / 2],
                samples[(samples.len() - 1) * 95 / 100],
                samples.last().unwrap(),
                waits[(waits.len() - 1) * 95 / 100],
                waits.last().unwrap()
            );
            eprintln!(
                "SQL round-trip {name} concurrency={concurrency}: p50={:.2} p95={:.2} max={:.2} (EXPLAIN JSON transfer included)",
                round_trips[round_trips.len() / 2],
                round_trips[(round_trips.len() - 1) * 95 / 100],
                round_trips.last().unwrap()
            );
            if *samples.last().unwrap() > 100. {
                violations.push(format!(
                    "{name}/{concurrency}: {:.2}ms",
                    samples.last().unwrap()
                ));
            }
        }
    }
    anyhow::ensure!(
        violations.is_empty(),
        "100ms budget failures: {}",
        violations.join(", ")
    );
    Ok(())
}

fn is_budget_write(name: &str) -> bool {
    matches!(name, "receipt_insert" | "post_delete")
}

type BudgetConnection =
    std::sync::Arc<tokio::sync::Mutex<(tokio_postgres::Client, std::collections::HashSet<String>)>>;
async fn budget_samples(
    pool: std::sync::Arc<Vec<BudgetConnection>>,
    worker: usize,
    name: String,
    sql: String,
    args: Vec<String>,
) -> anyhow::Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let execution = format!(
        "EXPLAIN (ANALYZE,TIMING OFF,FORMAT JSON) EXECUTE budget_{}({})",
        name,
        args.iter()
            .map(|v| format!("'{}'", v.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut samples = Vec::new();
    let mut waits = Vec::new();
    let mut round_trips = Vec::new();
    for _ in 0..30 {
        let waiting = std::time::Instant::now();
        let mut connection = pool[worker % pool.len()].lock().await;
        waits.push(waiting.elapsed().as_secs_f64() * 1000.);
        connection
            .0
            .batch_execute(
                if matches!(name.as_str(), "detail" | "thread" | "page_render") {
                    "SET plan_cache_mode=force_generic_plan"
                } else {
                    "SET plan_cache_mode=force_custom_plan"
                },
            )
            .await?;
        let mut prepare_ms = 0.;
        let mut prepare_round_trip_ms = 0.;
        if !connection.1.contains(&name) {
            let prepare = format!(
                "PREPARE budget_{}(text,text,text,text,text,text,text,text,text) AS {sql}",
                name
            );
            // Time preparation inside PostgreSQL, including dynamic SQL parsing.
            // Client scheduler/transport delay is reported separately.
            let command = format!(
                "DO $budget$ DECLARE started timestamptz:=clock_timestamp(); BEGIN EXECUTE '{}'; PERFORM set_config('zoo.bench_prepare_ms',(extract(epoch FROM clock_timestamp()-started)*1000)::text,false); END $budget$; SELECT current_setting('zoo.bench_prepare_ms');",
                prepare.replace('\'', "''")
            );
            let start = std::time::Instant::now();
            let rows = connection.0.simple_query(&command).await?;
            prepare_ms = rows
                .iter()
                .filter_map(|r| match r {
                    SimpleQueryMessage::Row(row) => row.get(0),
                    _ => None,
                })
                .last()
                .unwrap()
                .parse::<f64>()?;
            prepare_round_trip_ms = start.elapsed().as_secs_f64() * 1000.;
            eprintln!(
                "prepare {name}: server={prepare_ms:.3}ms round-trip={:.3}ms",
                prepare_round_trip_ms
            );
            connection.1.insert(name.clone());
        }
        // Roll back writes so every sample exercises the same actual change,
        // including receipt metadata, reply deltas and their lock contention.
        if is_budget_write(&name) {
            connection.0.batch_execute("BEGIN").await?;
        }
        let started = std::time::Instant::now();
        let rows = connection.0.simple_query(&execution).await?;
        round_trips.push(prepare_round_trip_ms + started.elapsed().as_secs_f64() * 1000.);
        if is_budget_write(&name) {
            connection.0.batch_execute("ROLLBACK").await?;
        }
        drop(connection);
        let raw = rows
            .iter()
            .find_map(|r| match r {
                SimpleQueryMessage::Row(row) => row.get(0),
                _ => None,
            })
            .unwrap();
        let plan: Value = serde_json::from_str(raw)?;
        if name == "page_render" {
            assert_eq!(plan[0]["Plan"]["Actual Rows"], 500);
            visit(&plan[0]["Plan"], &|node| {
                if node["Function Name"] == "jsonb_array_elements_text" {
                    assert!(
                        node["Actual Loops"].as_f64().unwrap() <= 1.,
                        "request IDs must be parsed once, not once per row"
                    );
                }
            });
        }
        samples.push(
            prepare_ms
                + plan[0]["Planning Time"].as_f64().unwrap()
                + plan[0]["Execution Time"].as_f64().unwrap(),
        );
    }
    Ok((samples, waits, round_trips))
}

#[tokio::test]
#[ignore = "repeat the strict budget gate on the committed SCALE_BUDGET fixture"]
async fn persisted_query_budget() -> anyhow::Result<()> {
    let (client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let row = client
        .query_one(
            "SELECT user_id,parent_id FROM apub_reply_counts WHERE n>0 LIMIT 1",
            &[],
        )
        .await?;
    verify_budget(&row.get::<_, String>(0), &row.get::<_, String>(1)).await
}

#[tokio::test]
#[ignore = "measure the complete handler on a committed scale fixture"]
async fn persisted_home_handler_budget() -> anyhow::Result<()> {
    let (client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    client
        .execute(
            "INSERT INTO role(id,role_name) VALUES('role_admin','admin') ON CONFLICT DO NOTHING",
            &[],
        )
        .await?;
    let user: String = client
        .query_one(
            "SELECT user_id FROM apub_reply_counts WHERE n>0 LIMIT 1",
            &[],
        )
        .await?
        .get(0);
    benchmark_home_requests(&user, 1000000).await
}
