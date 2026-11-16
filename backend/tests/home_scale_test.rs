//! Opt-in scale regression: uses a migrated disposable PostgreSQL database.
//! SCALE_POSTS=1000000 cargo test --test home_scale_test -- --ignored --nocapture
use backend::{
    config::Config,
    routes::home::{timeline_page_query, timeline_query},
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
            timeline_page_query(),
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
            timeline_page_query(),
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
            timeline_page_query(),
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
    // remain exact and therefore necessarily inspect its actual direct replies.
    let root = format!("{prefix}/posts/000000001");
    tx.execute(
        "UPDATE apub_remote_posts SET parent_id=$1 WHERE actor_id=$2 AND id<>$1",
        &[&root, &actor],
    )
    .await?;
    for thread in ["false", "true"] {
        let sql = bind(
            timeline_query(),
            &[&user, "", "social", "false", "500", &root, "", thread, "[]"],
        );
        let p = plan(&tx, &sql).await?;
        visit(&p["Plan"], &|n| {
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
            .map(|s| serde_json::from_str(s).unwrap())
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
    tx.rollback().await?;
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
