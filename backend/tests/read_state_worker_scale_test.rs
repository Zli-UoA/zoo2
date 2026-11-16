//! SCALE_POSTS=300000/1000000; use an empty migrated disposable database.
use backend::{
    config::Config, display_refresh::reservation_sql, routes::read_states::states_sql,
    slack::sync::claim_sql,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_postgres::{NoTls, SimpleQueryMessage};

fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
fn rows(messages: &[SimpleQueryMessage]) -> Vec<&str> {
    messages
        .iter()
        .filter_map(|r| match r {
            SimpleQueryMessage::Row(r) => r.get(0),
            _ => None,
        })
        .collect()
}
fn walk(node: &Value, f: &mut impl FnMut(&Value)) {
    f(node);
    if let Some(children) = node["Plans"].as_array() {
        for c in children {
            walk(c, f)
        }
    }
}
fn work(name: &str, plan: &Value) -> Value {
    let mut relations = std::collections::BTreeMap::<String, f64>::new();
    let mut max_sort = 0_f64;
    let mut max_intermediate = 0_f64;
    walk(&plan["Plan"], &mut |node| {
        let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
        let n = (node["Actual Rows"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
            * loops;
        max_intermediate = max_intermediate
            .max(n + node["Rows Removed by Join Filter"].as_f64().unwrap_or(0.) * loops);
        if let Some(table) = node["Relation Name"].as_str() {
            *relations.entry(table.into()).or_default() += n;
        }
        if node["Node Type"] == "Sort" {
            let input: f64 = node["Plans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    c["Actual Rows"].as_f64().unwrap_or(0.)
                        * c["Actual Loops"].as_f64().unwrap_or(0.)
                })
                .sum();
            max_sort = max_sort.max(input);
        }
    });
    if name != "legacy_read_states" {
        for (table, n) in &relations {
            assert!(
                *n <= 128.,
                "{name}: {table} scanned {n} rows: {relations:?}"
            );
        }
        assert!(
            max_intermediate <= 128.,
            "{name}: unbounded intermediate work {max_intermediate}"
        );
        assert!(max_sort <= 128., "{name}: unbounded sort input {max_sort}");
        assert_eq!(
            plan["Plan"]["Temp Written Blocks"].as_u64().unwrap_or(0),
            0,
            "{name}: disk spill"
        );
        if name.starts_with("read_") {
            assert_eq!(
                *relations.get("messages").unwrap_or(&0.),
                0.,
                "unread must not enumerate message history"
            );
            assert!(*relations.get("channel_message_authors").unwrap_or(&0.) <= 6.);
        }
    }
    json!({"relations":relations,"sort_input":max_sort,"intermediate":max_intermediate,"logical_blocks":plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)+plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0)})
}

#[tokio::test]
#[ignore = "inserts 300k/1m messages, import jobs and queue entries"]
async fn read_states_and_worker_claims_do_not_scan_history() -> anyhow::Result<()> {
    let count: usize = std::env::var("SCALE_POSTS")
        .unwrap_or_else(|_| "300000".into())
        .parse()?;
    assert!(count >= 300000);
    let url = Config::from_env().database_url;
    let (mut client, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    client
        .batch_execute("SET jit=off; SET statement_timeout='300s'")
        .await?;
    let user = uuid::Uuid::new_v4().to_string();
    let author = uuid::Uuid::new_v4().to_string();
    let channels: Vec<String> = (0..3).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let team = format!("{user}-team");
    let unrelated = uuid::Uuid::new_v4().to_string();
    let extra = (count / 10) as i32;
    let own_count = count - extra as usize;
    let tx = client.transaction().await?;
    tx.execute("INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",&[&user,&author]).await?;
    for channel in &channels {
        tx.execute(
            "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
            &[channel, &user],
        )
        .await?;
        tx.execute(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
            &[&user, channel],
        )
        .await?;
        tx.execute("INSERT INTO channel_read_states(user_id,channel_id,last_read_at) VALUES($1,$2,'2024-01-01')",&[&user,channel]).await?;
    }
    // Generate old history without synchronous display/summary work; populate
    // exact watermarks afterward. Every measured write uses all real triggers.
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&unrelated, &user],
    )
    .await?;
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT $1||'-message-'||lpad(n::text,9,'0'),$1,($2::text[])[1+n%3],'fixture',timestamp '2025-01-01'+n*interval '1 millisecond' FROM generate_series(1,$3::integer)n", &[&user,&channels,&(own_count as i32)]).await?;
    tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1||'-old-other',$2,$3,'old','2000-01-01'),($1||'-new-other',$2,$4,'new','2030-01-01')", &[&user,&author,&channels[1],&channels[2]]).await?;
    tx.execute("INSERT INTO users(id,user_name,display_name) SELECT $1||'-unrelated-'||n,$1||'-unrelated-'||n,'Other' FROM generate_series(1,$2::integer)n", &[&user,&extra]).await?;
    tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT $1||'-unrelated-message-'||n,$1||'-unrelated-'||n,$2,'other',timestamp '2025-01-01' FROM generate_series(1,$3::integer)n", &[&user,&unrelated,&extra]).await?;
    let tasks = (count / 1000) as i32;
    tx.execute("INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) SELECT $1,'C'||n,$2,'1' FROM generate_series(0,$3::integer-1)n", &[&team,&channels[0],&tasks]).await?;
    tx.execute("INSERT INTO slack_channel_syncs(team_id,slack_channel_id,available_at) SELECT $1,'C'||n,'2035-01-01' FROM generate_series(0,$2::integer-1)n", &[&team,&tasks]).await?;
    tx.execute("INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id) SELECT $1||'-import-'||n,$1,CASE WHEN n%100=0 THEN 'analyzing' WHEN n%10=0 THEN 'fetching_files' ELSE 'completed' END,'fixture',0,'fixture',CASE WHEN n%100=0 THEN $2||'-other-team' ELSE $2 END FROM generate_series(1,$3::integer)n", &[&user,&team,&(count as i32)]).await?;
    tx.execute("INSERT INTO display_refresh_queue(kind,target,retry_at) SELECT CASE WHEN n%2=0 THEN 'message' ELSE 'emoji' END,$1||'-queue-'||n,'2035-01-01' FROM generate_series(1,$2::integer)n", &[&user,&(count as i32)]).await?;
    tx.batch_execute("SET LOCAL session_replication_role=origin")
        .await?;
    tx.batch_execute("INSERT INTO channel_message_authors(channel_id,user_id,latest_at) SELECT channel_id,user_id,max(created_at) FROM messages WHERE parent_id IS NULL AND deleted_at IS NULL GROUP BY channel_id,user_id ON CONFLICT(channel_id,user_id) DO UPDATE SET latest_at=EXCLUDED.latest_at").await?;
    tx.commit().await?;
    // The fixed event catalog is populated by migrations. Analyze it explicitly:
    // autovacuum may otherwise switch its lookup from index to sequential scan
    // between scales (1 vs 90 rows), which is unrelated to message history.
    client.batch_execute("ANALYZE app_event_types;").await?;
    client.batch_execute("ANALYZE messages; ANALYZE users; ANALYZE channels; ANALYZE channel_user; ANALYZE channel_read_states; ANALYZE channel_message_authors; ANALYZE slack_import_jobs; ANALYZE slack_channels; ANALYZE slack_channel_syncs; ANALYZE slack_channel_wakes; ANALYZE display_refresh_queue;").await?;
    let output = std::env::var_os("SCALE_PLAN_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir)?;
    }
    // The original correlated OR walks the viewer's own long history too.
    let legacy = format!(
        "SELECT json_build_object('channel_id',c.id,'last_read_at',r.last_read_at,'has_unread',EXISTS(SELECT 1 FROM messages m WHERE m.channel_id=c.id AND m.parent_id IS NULL AND m.deleted_at IS NULL AND m.user_id<>{u} AND(r.last_read_at IS NULL OR m.created_at>r.last_read_at)))::text FROM channels c LEFT JOIN channel_read_states r ON r.channel_id=c.id AND r.user_id={u} WHERE c.deleted_at IS NULL AND EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id={u})",
        u = quoted(&user)
    );
    let raw = client
        .simple_query(&format!(
            "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {legacy}"
        ))
        .await?;
    let plan: Value = serde_json::from_str(rows(&raw)[0])?;
    eprintln!(
        "{count} legacy work={} planning={} execution={}",
        work("legacy_read_states", &plan[0]),
        plan[0]["Planning Time"],
        plan[0]["Execution Time"]
    );
    if let Some(dir) = &output {
        std::fs::write(dir.join("legacy.json"), serde_json::to_vec_pretty(&plan)?)?;
    }
    client
        .batch_execute("BEGIN; DROP INDEX slack_import_jobs_active_team")
        .await?;
    client.execute("UPDATE slack_channel_syncs SET available_at='2020-01-01' WHERE team_id=$1 AND slack_channel_id='C0'", &[&team]).await?;
    client.execute("INSERT INTO slack_channel_wakes(team_id,slack_channel_id,first_at) VALUES($1,'C1',now()-interval '10 seconds')", &[&team]).await?;
    let legacy_claim = include_str!("fixtures/legacy_slack_claim.sql").replace("$1", "'baseline'");
    let raw = client
        .simple_query(&format!(
            "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {legacy_claim}"
        ))
        .await?;
    let plan: Value = serde_json::from_str(rows(&raw)[0])?;
    eprintln!(
        "{count} legacy Slack planning={} execution={}; plan saved",
        plan[0]["Planning Time"], plan[0]["Execution Time"]
    );
    if let Some(dir) = &output {
        std::fs::write(
            dir.join("legacy-slack.json"),
            serde_json::to_vec_pretty(&plan)?,
        )?;
    }
    client.batch_execute("ROLLBACK").await?;
    let sql = states_sql(false).replace("$1", &quoted(&user));
    let current = client.simple_query(&sql).await?;
    let values: Vec<Value> = rows(&current)
        .iter()
        .map(|r| serde_json::from_str(r).unwrap())
        .collect();
    for (i, channel) in channels.iter().enumerate() {
        assert_eq!(
            values.iter().find(|r| r["channel_id"] == *channel).unwrap()["has_unread"],
            i == 2
        );
    }
    let queries = vec![
        ("read_list", states_sql(false), vec![user.clone()], false),
        (
            "read_single",
            states_sql(true),
            vec![user.clone(), channels[0].clone()],
            false,
        ),
        (
            "slack_wake_empty",
            claim_sql(true),
            vec![uuid::Uuid::new_v4().to_string()],
            true,
        ),
        (
            "slack_due_empty",
            claim_sql(false),
            vec![uuid::Uuid::new_v4().to_string()],
            true,
        ),
        ("display_row_empty", reservation_sql(false), vec![], true),
        ("display_scope_empty", reservation_sql(true), vec![], true),
    ];
    let mut metrics = serde_json::Map::new();
    for (name, sql, args, write) in queries {
        measure(&url, name, sql, args, write, &output, &mut metrics).await?;
    }
    client.execute("INSERT INTO slack_channel_wakes(team_id,slack_channel_id,first_at) VALUES($1,'C1',now()-interval '10 seconds')", &[&team]).await?;
    client.execute("UPDATE slack_channel_syncs SET available_at='2020-01-01' WHERE team_id=$1 AND slack_channel_id='C0'", &[&team]).await?;
    client
        .execute(
            "INSERT INTO display_refresh_queue(kind,target) VALUES('message',$1),('emoji',$1)",
            &[&user],
        )
        .await?;
    client.batch_execute("ANALYZE slack_channel_wakes; ANALYZE slack_channel_syncs; ANALYZE display_refresh_queue").await?;
    let queries=vec![
        ("slack_wake_ready",claim_sql(true),vec![uuid::Uuid::new_v4().to_string()]),
        ("slack_due_ready",claim_sql(false),vec![uuid::Uuid::new_v4().to_string()]),
        ("display_row_ready",reservation_sql(false),vec![]),
        ("display_scope_ready",reservation_sql(true),vec![]),
        ("message_insert", "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'probe','2100-01-01') RETURNING id".into(),vec![format!("{user}-probe"),user.clone(),channels[0].clone()]),
        ("message_delete_latest","DELETE FROM messages WHERE id=$1 RETURNING id".into(),vec![format!("{user}-message-{own_count:09}")]),
    ];
    for (name, sql, args) in queries {
        measure(&url, name, sql, args, true, &output, &mut metrics).await?;
    }
    if let Some(dir) = &output {
        std::fs::write(dir.join("work.json"), serde_json::to_vec_pretty(&metrics)?)?;
    }
    measure_api(&user, &channels[0]).await?;
    Ok(())
}

fn merge_work(target: &mut Option<Value>, next: Value) {
    if let Some(current) = target {
        for (key, value) in next["relations"].as_object().unwrap() {
            let maximum = current["relations"][key]
                .as_f64()
                .unwrap_or(0.)
                .max(value.as_f64().unwrap());
            current["relations"][key] = json!(maximum);
        }
        for key in ["sort_input", "intermediate", "logical_blocks"] {
            current[key] = json!(
                current[key]
                    .as_f64()
                    .unwrap()
                    .max(next[key].as_f64().unwrap())
            );
        }
        if let Some(nested) = next.get("nested") {
            let mut merged = current.get("nested").cloned();
            merge_work(&mut merged, nested.clone());
            current["nested"] = merged.unwrap();
        }
        if let Some(statements) = next["statements"].as_u64() {
            current["statements"] =
                json!(current["statements"].as_u64().unwrap_or(0).max(statements));
        }
    } else {
        *target = Some(next);
    }
}

fn nested_work(name: &str, plans: &[Value]) -> Value {
    assert!(
        !plans.is_empty() && plans.len() <= 128,
        "{name}: missing or unbounded nested statements: {}",
        plans.len()
    );
    assert!(
        plans
            .iter()
            .any(|p| p["Query Text"].as_str().is_some_and(|sql| sql.contains(
                "FROM messages WHERE channel_id=item.channel_id AND user_id=item.user_id"
            ))),
        "{name}: author watermark source plan was not captured"
    );
    let mut total = json!({"relations":{},"intermediate":0.,"sort_input":0.,"logical_blocks":0_u64,"statements":plans.len()});
    for plan in plans {
        // Each auto_explain notice is a real executed plan, including AFTER
        // trigger SQL missing from the parent EXPLAIN's node instrumentation.
        let metrics = work(&format!("{name}_nested"), plan);
        for (relation, rows) in metrics["relations"].as_object().unwrap() {
            total["relations"][relation] =
                json!(total["relations"][relation].as_f64().unwrap_or(0.) + rows.as_f64().unwrap());
        }
        for key in ["intermediate", "sort_input"] {
            total[key] = json!(
                total[key]
                    .as_f64()
                    .unwrap()
                    .max(metrics[key].as_f64().unwrap())
            );
        }
        total["logical_blocks"] = json!(
            total["logical_blocks"].as_u64().unwrap() + metrics["logical_blocks"].as_u64().unwrap()
        );
    }
    for (relation, rows) in total["relations"].as_object().unwrap() {
        assert!(
            rows.as_f64().unwrap() <= 512.,
            "{name}: nested {relation} work exceeds fixture budget: {rows}"
        );
    }
    total
}

#[test]
#[should_panic(expected = "messages scanned 300000 rows")]
fn nested_history_scan_is_rejected_when_parent_returns_one_row() {
    let parent = json!({"Plan":{"Node Type":"Insert","Relation Name":"messages","Actual Rows":1,"Actual Loops":1}});
    assert_eq!(work("message_insert", &parent)["relations"]["messages"], 1.);
    let source = json!({
        "Query Text":"SELECT created_at FROM messages WHERE channel_id=item.channel_id AND user_id=item.user_id",
        "Plan":{"Node Type":"Index Scan","Relation Name":"messages","Actual Rows":1,"Actual Loops":1}
    });
    let history = json!({
        "Query Text":"SELECT count(*) FROM messages",
        "Plan":{"Node Type":"Aggregate","Actual Rows":1,"Actual Loops":1,"Plans":[
            {"Node Type":"Seq Scan","Relation Name":"messages","Actual Rows":300000,"Actual Loops":1}
        ]}
    });
    nested_work("message_insert", &[source, history]);
}

async fn measure(
    url: &str,
    name: &str,
    sql: String,
    args: Vec<String>,
    write: bool,
    output: &Option<std::path::PathBuf>,
    metrics: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut observed = None;
    for concurrency in [1, 8] {
        let mut tasks = tokio::task::JoinSet::new();
        for worker in 0..concurrency {
            let url = url.to_owned();
            let sql = sql.clone();
            let name = name.to_owned();
            let mut args = args.clone();
            let output = output.clone();
            if name == "message_insert" {
                args[0] = format!("{}-{worker}", args[0]);
            }
            tasks.spawn(async move {
                let connected = std::time::Instant::now();
                let (client, connection) = tokio_postgres::connect(&url, NoTls).await?;
                let connection_ms = connected.elapsed().as_secs_f64() * 1000.;
                let nested = Arc::new(Mutex::new(Vec::<Value>::new()));
                let captured = nested.clone();
                tokio::spawn(async move {
                    let mut connection = connection;
                    while let Some(message) = futures_util::future::poll_fn(|cx| connection.poll_message(cx)).await {
                        if let tokio_postgres::AsyncMessage::Notice(notice) = message?
                            && let Some((_, plan)) = notice.message().split_once("plan:\n")
                        {
                            let mut plans = captured.lock().unwrap();
                            assert!(plans.len() < 128, "unbounded auto_explain statements");
                            plans.push(serde_json::from_str(plan).expect("auto_explain JSON plan"));
                        }
                    }
                    Ok::<_, tokio_postgres::Error>(())
                });
                client
                    .batch_execute(if concurrency == 1 {
                        "SET jit=off;SET plan_cache_mode=force_custom_plan"
                    } else {
                        "SET jit=off;SET plan_cache_mode=force_generic_plan"
                    })
                    .await?;
                // Explicit argument types also permit unused parameters; this keeps
                // every production variant on the same preparation path.
                let started = std::time::Instant::now();
                client
                    .batch_execute(&format!(
                        "PREPARE probe(text,text,text,text,text,text,text,text,text) AS {sql}"
                    ))
                    .await?;
                let prepare = started.elapsed().as_secs_f64() * 1000.;
                let capture_nested = matches!(name.as_str(), "message_insert" | "message_delete_latest");
                if capture_nested {
                    // Test-only superuser session. NOTICE transports the same
                    // auto_explain JSON as server logs without global settings.
                    client.batch_execute("LOAD 'auto_explain'; SET client_min_messages=notice; SET auto_explain.log_level=notice; SET auto_explain.log_format=json; SET auto_explain.log_nested_statements=on; SET auto_explain.log_analyze=on; SET auto_explain.log_buffers=on; SET auto_explain.log_timing=off; SET auto_explain.sample_rate=1; SET auto_explain.log_min_duration=0").await?;
                }
                args.resize(9, String::new());
                let args = args.iter().map(|a| quoted(a)).collect::<Vec<_>>().join(",");
                let mut times = Vec::new();
                let mut roundtrips = Vec::new();
                let mut sample_work = None;
                for sample in 0..30 {
                    nested.lock().unwrap().clear();
                    if write {
                        client.batch_execute("BEGIN").await?;
                    }
                    let started = std::time::Instant::now();
                    let raw = client
                        .simple_query(&format!(
                            "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) EXECUTE probe({args})"
                        ))
                        .await?;
                    roundtrips.push(started.elapsed().as_secs_f64() * 1000.);
                    if write {
                        client.batch_execute("ROLLBACK").await?;
                    }
                    let plan: Value = serde_json::from_str(rows(&raw)[0])?;
                    let mut measured = work(&name, &plan[0]);
                    let nested_plans = std::mem::take(&mut *nested.lock().unwrap());
                    if capture_nested {
                        measured["nested"] = nested_work(&name, &nested_plans);
                    }
                    if sample == 0 && worker == 0 && let Some(dir) = &output {
                            std::fs::write(
                                dir.join(format!("{name}-{concurrency}.json")),
                                serde_json::to_vec_pretty(&plan)?,
                            )?;
                            if capture_nested {
                                std::fs::write(dir.join(format!("{name}-{concurrency}-nested.json")), serde_json::to_vec_pretty(&nested_plans)?)?;
                            }
                    }
                    merge_work(&mut sample_work, measured);
                    times.push(
                        plan[0]["Planning Time"].as_f64().unwrap()
                            + plan[0]["Execution Time"].as_f64().unwrap(),
                    );
                }
                roundtrips.sort_by(f64::total_cmp);
                eprintln!(
                    "{name}/{concurrency}/worker{worker}: connection={connection_ms:.3} first-plan-exec={:.3} roundtrip-p50={:.3} roundtrip-p95={:.3} roundtrip-max={:.3}",
                    times[0], roundtrips[roundtrips.len()/2], roundtrips[(roundtrips.len()-1)*95/100], roundtrips.last().unwrap()
                );
                Ok::<_, anyhow::Error>((times, sample_work, prepare))
            });
        }
        let mut times = Vec::new();
        let mut prepares = Vec::new();
        while let Some(result) = tasks.join_next().await {
            let (samples, w, prepare) = result??;
            times.extend(samples);
            prepares.push(prepare);
            if let Some(w) = w {
                merge_work(&mut observed, w);
            }
        }
        times.sort_by(f64::total_cmp);
        prepares.sort_by(f64::total_cmp);
        eprintln!(
            "{name}/{concurrency}: samples={} p50={:.3} p95={:.3} max={:.3} preparation-roundtrip-max={:.3}; work={}",
            times.len(),
            times[times.len() / 2],
            times[(times.len() - 1) * 95 / 100],
            times.last().unwrap(),
            prepares.last().unwrap(),
            observed.as_ref().unwrap()
        );
    }
    metrics.insert(name.into(), observed.unwrap());
    Ok(())
}

async fn measure_api(user: &str, channel: &str) -> anyhow::Result<()> {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let started = std::time::Instant::now();
    let db = backend::db::init_db(&config.database_url).await?;
    let app = backend::routes::create_router(backend::state::AppState::new(db, config));
    eprintln!(
        "API pool/router initialization={:.3}ms",
        started.elapsed().as_secs_f64() * 1000.
    );
    for path in [
        "/api/channel-read-states".to_owned(),
        format!("/api/channels/{channel}/read-state"),
    ] {
        for concurrency in [1, 8] {
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..concurrency {
                let app = app.clone();
                let user = user.to_owned();
                let path = path.clone();
                tasks.spawn(async move {
                    let mut times = Vec::new();
                    for _ in 0..30 {
                        let started = std::time::Instant::now();
                        let response = app
                            .clone()
                            .oneshot(
                                Request::builder()
                                    .uri(&path)
                                    .header("X-Dev-User", &user)
                                    .body(Body::empty())?,
                            )
                            .await?;
                        assert_eq!(response.status(), StatusCode::OK);
                        let body = to_bytes(response.into_body(), 1024 * 1024).await?;
                        let _: Value = serde_json::from_slice(&body)?;
                        times.push(started.elapsed().as_secs_f64() * 1000.);
                    }
                    eprintln!("API {path}/{concurrency}: first={:.3}ms", times[0]);
                    Ok::<_, anyhow::Error>(times)
                });
            }
            let mut times = Vec::new();
            while let Some(result) = tasks.join_next().await {
                times.extend(result??);
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "API {path}/{concurrency}: samples={} p50={:.3} p95={:.3} max={:.3}ms (in-process, includes pool wait/auth/transaction/serialization)",
                times.len(),
                times[times.len() / 2],
                times[(times.len() - 1) * 95 / 100],
                times.last().unwrap()
            );
        }
    }
    Ok(())
}
