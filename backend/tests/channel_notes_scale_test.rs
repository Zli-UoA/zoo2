//! SCALE_NOTES=300000/1000000; use an empty migrated disposable database.
//! Measures the note API's target-channel work while unrelated note history grows.
use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio_postgres::NoTls;

const TARGET_NOTES: usize = 50;

fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn walk(node: &Value, f: &mut impl FnMut(&Value)) {
    f(node);
    if let Some(children) = node["Plans"].as_array() {
        for child in children {
            walk(child, f);
        }
    }
}

fn work(plan: &Value, bound: f64) -> Value {
    let mut relations = BTreeMap::<String, f64>::new();
    let mut max_sort = 0_f64;
    let mut max_intermediate = 0_f64;
    walk(&plan["Plan"], &mut |node| {
        let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
        let rows = (node["Actual Rows"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
            * loops;
        max_intermediate = max_intermediate.max(rows);
        if let Some(table) = node["Relation Name"].as_str() {
            *relations.entry(table.into()).or_default() += rows;
        }
        if node["Node Type"] == "Sort" {
            let input: f64 = node["Plans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|child| {
                    child["Actual Rows"].as_f64().unwrap_or(0.)
                        * child["Actual Loops"].as_f64().unwrap_or(0.)
                })
                .sum();
            max_sort = max_sort.max(input);
        }
    });
    let total: f64 = relations.values().sum();
    assert!(
        total <= bound,
        "unrelated note history was scanned: {relations:?} (bound {bound})"
    );
    assert!(
        max_intermediate <= bound,
        "unbounded intermediate work {max_intermediate} (bound {bound})"
    );
    assert!(max_sort <= bound, "unbounded sort input {max_sort}");
    assert_eq!(
        plan["Plan"]["Temp Written Blocks"].as_u64().unwrap_or(0),
        0,
        "disk spill"
    );
    json!({
        "relations": relations,
        "intermediate": max_intermediate,
        "sort_input": max_sort,
        "logical_blocks": plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)
            + plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0),
    })
}

async fn measure(
    url: &str,
    name: &str,
    sql: &str,
    args: &[&str],
    bound: f64,
    output: &Option<std::path::PathBuf>,
    metrics: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut max = None;
    for concurrency in [1, 8] {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let url = url.to_owned();
            let sql = sql.to_owned();
            let args = args.iter().map(|value| quoted(value)).collect::<Vec<_>>();
            tasks.spawn(async move {
                let (client, connection) = tokio_postgres::connect(&url, NoTls).await?;
                tokio::spawn(connection);
                client
                    .batch_execute(if concurrency == 1 {
                        "SET jit=off;SET plan_cache_mode=force_custom_plan"
                    } else {
                        "SET jit=off;SET plan_cache_mode=force_generic_plan"
                    })
                    .await?;
                client
                    .batch_execute(&format!("PREPARE probe(text,text,text,text,text) AS {sql}"))
                    .await?;
                let mut bind = args.clone();
                bind.resize(5, String::from("''"));
                let mut observed = None;
                for _ in 0..30 {
                    client.batch_execute("BEGIN").await?;
                    let raw = client
                        .simple_query(&format!(
                            "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) EXECUTE probe({})",
                            bind.join(",")
                        ))
                        .await?;
                    client.batch_execute("ROLLBACK").await?;
                    let plan: Value = serde_json::from_str(
                        raw.iter()
                            .find_map(|message| match message {
                                tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0),
                                _ => None,
                            })
                            .expect("EXPLAIN row"),
                    )?;
                    let current = work(&plan[0], bound);
                    observed = Some(match observed {
                        Some(previous) => merge_work(previous, current),
                        None => current,
                    });
                }
                Ok::<_, anyhow::Error>(observed.expect("measurement"))
            });
        }
        while let Some(value) = tasks.join_next().await {
            let value = value??;
            max = Some(match max {
                Some(previous) => merge_work(previous, value),
                None => value,
            });
        }
    }
    let max = max.expect("measurement");
    eprintln!(
        "{name}/{}: rows={} blocks={}",
        if bound > 512.0 { "cleanup" } else { "note" },
        max["relations"]
            .as_object()
            .unwrap()
            .values()
            .map(|value| value.as_f64().unwrap_or(0.))
            .sum::<f64>(),
        max["logical_blocks"]
    );
    if let Some(dir) = output {
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&max)?,
        )?;
    }
    metrics.insert(name.into(), max);
    Ok(())
}

fn merge_work(mut current: Value, next: Value) -> Value {
    for (key, value) in next["relations"].as_object().unwrap() {
        let previous = current["relations"][key].as_f64().unwrap_or(0.);
        current["relations"][key] = json!(previous.max(value.as_f64().unwrap_or(0.)));
    }
    for key in ["intermediate", "sort_input", "logical_blocks"] {
        let previous = current[key].as_f64().unwrap_or(0.);
        current[key] = json!(previous.max(next[key].as_f64().unwrap_or(0.)));
    }
    current
}

#[tokio::test]
#[ignore = "inserts 300k/1m unrelated notes; use an empty migrated database"]
async fn channel_notes_work_is_bounded_by_channel_notes_not_history() -> anyhow::Result<()> {
    let count: usize = std::env::var("SCALE_NOTES")
        .unwrap_or_else(|_| "300000".into())
        .parse()?;
    assert!(count >= 300000);
    let url = backend::config::Config::from_env().database_url;
    let (mut client, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    client
        .batch_execute("SET jit=off; SET statement_timeout='300s'")
        .await?;

    let owner = uuid::Uuid::new_v4().to_string();
    let target_active = uuid::Uuid::new_v4().to_string();
    let target_empty = uuid::Uuid::new_v4().to_string();
    let target_deleted = uuid::Uuid::new_v4().to_string();
    let target_writes = uuid::Uuid::new_v4().to_string();
    let tx = client.transaction().await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Notes scale')",
        &[&owner],
    )
    .await?;
    for (id, name) in [
        (&target_active, "active"),
        (&target_empty, "empty"),
        (&target_deleted, "deleted"),
        (&target_writes, "writes"),
    ] {
        tx.execute(
            "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false)",
            &[id, &format!("notes_scale_{name}_{}", uuid::Uuid::new_v4().simple()), &owner],
        )
        .await?;
        tx.execute(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
            &[&owner, id],
        )
        .await?;
    }
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    for target in [&target_active, &target_writes] {
        tx.execute(
            "INSERT INTO channel_notes(id,channel_id,title,content,position,version,created_user_id,created_at,updated_at) \
             SELECT 'note-'||$1||'-'||n,$1,'title','content',n,1,$2,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC' \
             FROM generate_series(0,$3::integer-1)n",
            &[target, &owner, &(TARGET_NOTES as i32)],
        )
        .await?;
    }
    tx.execute(
        "INSERT INTO channel_notes(id,channel_id,title,content,position,deleted_at,version,created_user_id,created_at,updated_at) \
         SELECT 'note-'||$1||'-'||n,$1,'title','content',n, \
           CASE WHEN n < $2 THEN now() AT TIME ZONE 'UTC' - interval '1 hour' \
                ELSE now() AT TIME ZONE 'UTC' - interval '2 days' END, \
           1,$3,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC' \
         FROM generate_series(0,$4::integer-1)n",
        &[
            &target_deleted,
            &(TARGET_NOTES as i32),
            &owner,
            &((TARGET_NOTES * 2) as i32),
        ],
    )
    .await?;
    let other_channels = (count / 100).max(100);
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) \
         SELECT 'other-channel-'||lpad(n::text,9,'0'),'other_'||n,$1,false \
         FROM generate_series(1,$2::integer)n",
        &[&owner, &(other_channels as i32)],
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_notes(id,channel_id,title,content,position,deleted_at,version,created_user_id,created_at,updated_at) \
         SELECT 'other-note-'||lpad(n::text,9,'0'), \
           'other-channel-'||lpad((((n-1)/100)+1)::text,9,'0'), \
           'title','content',(n-1)%100, \
           CASE WHEN n % 10 = 0 THEN now() AT TIME ZONE 'UTC' - interval '2 days' ELSE NULL END, \
           1,$1,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC' \
         FROM generate_series(1,$2::integer)n",
        &[&owner, &(count as i32)],
    )
    .await?;
    tx.batch_execute("SET LOCAL session_replication_role=origin")
        .await?;
    tx.commit().await?;
    client.batch_execute("ANALYZE").await?;

    let output = std::env::var_os("SCALE_PLAN_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir)?;
    }
    let active_note = format!("note-{target_active}-0");
    let deleted_note = format!("note-{target_deleted}-0");
    let mut metrics = serde_json::Map::new();
    let list = format!(
        "SELECT id,channel_id,title,content,position,deleted_at,version,created_user_id,created_at,updated_at \
         FROM channel_notes WHERE channel_id={} AND deleted_at IS NULL",
        quoted(&target_active)
    );
    measure(&url, "notes_list", &list, &[], 256., &output, &mut metrics).await?;
    let list_empty = format!(
        "SELECT id,channel_id,title,content,position,deleted_at,version,created_user_id,created_at,updated_at \
         FROM channel_notes WHERE channel_id={} AND deleted_at IS NULL",
        quoted(&target_empty)
    );
    measure(
        &url,
        "notes_list_empty",
        &list_empty,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let list_deleted = format!(
        "SELECT id, channel_id, title, content, version, created_user_id, \
                to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS'), \
                to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS') \
         FROM channel_notes \
         WHERE channel_id = {} AND deleted_at IS NOT NULL \
           AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
         ORDER BY deleted_at, id",
        quoted(&target_deleted)
    );
    measure(
        &url,
        "notes_list_deleted",
        &list_deleted,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let show = format!(
        "SELECT id,channel_id,title,content,position,deleted_at,version,created_user_id,created_at,updated_at \
         FROM channel_notes WHERE id={} AND channel_id={} AND deleted_at IS NULL",
        quoted(&active_note),
        quoted(&target_active)
    );
    measure(&url, "notes_show", &show, &[], 256., &output, &mut metrics).await?;
    let position = format!(
        "SELECT COALESCE(MAX(position), -1) + 1 AS position \
         FROM channel_notes WHERE channel_id = {} AND deleted_at IS NULL",
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_position",
        &position,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let update = format!(
        "UPDATE channel_notes SET title = 'updated', content = 'updated', version = version + 1, \
         updated_at = statement_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = {} AND channel_id = {} AND version = 1 AND deleted_at IS NULL RETURNING id",
        quoted(&active_note),
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_update",
        &update,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let delete = format!(
        "UPDATE channel_notes SET deleted_at = statement_timestamp() AT TIME ZONE 'UTC', \
         version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = {} AND channel_id = {} AND version = 1 AND deleted_at IS NULL RETURNING id",
        quoted(&active_note),
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_delete",
        &delete,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let restore = format!(
        "UPDATE channel_notes SET deleted_at = NULL, version = version + 1, \
         updated_at = statement_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = {} AND channel_id = {} AND deleted_at IS NOT NULL \
           AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
         RETURNING position",
        quoted(&deleted_note),
        quoted(&target_deleted)
    );
    measure(
        &url,
        "notes_restore",
        &restore,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let reorder_update = format!(
        "UPDATE channel_notes SET position = 0 WHERE id = {} AND channel_id = {}",
        quoted(&active_note),
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_reorder_update",
        &reorder_update,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let cleanup_page = "WITH page AS MATERIALIZED (SELECT id FROM channels WHERE id > $1 ORDER BY id LIMIT $2::integer) \
         SELECT page.id, expired.id IS NOT NULL AS eligible FROM page \
         LEFT JOIN LATERAL (SELECT id FROM channel_notes WHERE channel_id = page.id \
           AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
           ORDER BY deleted_at, id LIMIT 1) expired ON true ORDER BY page.id";
    measure(
        &url,
        "notes_cleanup_page",
        cleanup_page,
        &["", "100"],
        2048.,
        &output,
        &mut metrics,
    )
    .await?;
    let cleanup_delete = format!(
        "DELETE FROM channel_notes WHERE id IN (\
         SELECT id FROM channel_notes WHERE channel_id = {} \
         AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
           ORDER BY deleted_at, id LIMIT $1::integer FOR UPDATE SKIP LOCKED)",
        quoted(&target_deleted)
    );
    measure(
        &url,
        "notes_cleanup_delete",
        &cleanup_delete,
        &["100"],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    let plaintext_check = format!(
        "SELECT encrypted FROM channels WHERE id = {} FOR SHARE",
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_plaintext_check",
        &plaintext_check,
        &[],
        16.,
        &output,
        &mut metrics,
    )
    .await?;
    let encrypt_check = format!(
        "SELECT EXISTS (SELECT 1 FROM channel_notes WHERE channel_id = {} \
         AND (deleted_at IS NULL OR deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'))",
        quoted(&target_active)
    );
    measure(
        &url,
        "notes_encrypt_check",
        &encrypt_check,
        &[],
        256.,
        &output,
        &mut metrics,
    )
    .await?;
    if let Some(dir) = &output {
        std::fs::write(dir.join("work.json"), serde_json::to_vec_pretty(&metrics)?)?;
    }
    Ok(())
}

async fn api_request(
    app: &axum::Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (axum::http::StatusCode, Value) {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", actor)
                .body(Body::from(if body.is_null() {
                    String::new()
                } else {
                    body.to_string()
                }))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn stats(mut times: Vec<f64>) -> Value {
    times.sort_by(f64::total_cmp);
    let p50 = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    let max = *times.last().unwrap();
    json!({"samples": times.len(), "p50_ms": p50, "p95_ms": p95, "max_ms": max})
}

async fn bench_static(
    app: &axum::Router,
    actor: &str,
    method: &str,
    path: &str,
    body: Value,
    concurrency: usize,
    name: &str,
    api: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut times = Vec::new();
    for _ in 0..30 {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let app = app.clone();
            let actor = actor.to_owned();
            let method = method.to_owned();
            let path = path.to_owned();
            let body = body.clone();
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                let (status, value) = api_request(&app, &method, &path, &actor, body).await;
                assert_eq!(status, axum::http::StatusCode::OK, "{value}");
                Ok::<_, anyhow::Error>(started.elapsed().as_secs_f64() * 1000.)
            });
        }
        while let Some(value) = tasks.join_next().await {
            times.push(value??);
        }
    }
    let summary = stats(times);
    eprintln!("API {name} c={concurrency}: {summary}");
    api.insert(format!("{name}/c{concurrency}"), summary);
    Ok(())
}

async fn bench_update(
    app: &axum::Router,
    actor: &str,
    writes: &str,
    ids: &[String],
    offset: usize,
    concurrency: usize,
    api: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut times = Vec::new();
    for round in 0..30 {
        let mut tasks = tokio::task::JoinSet::new();
        for worker in 0..concurrency {
            let app = app.clone();
            let actor = actor.to_owned();
            let path = format!("/api/channels/{writes}/notes/{}", ids[offset + worker]);
            let body = json!({"title":"updated","content":"updated","version":round + 1});
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                let (status, value) = api_request(&app, "PATCH", &path, &actor, body).await;
                assert_eq!(status, axum::http::StatusCode::OK, "{value}");
                Ok::<_, anyhow::Error>(started.elapsed().as_secs_f64() * 1000.)
            });
        }
        while let Some(value) = tasks.join_next().await {
            times.push(value??);
        }
    }
    let summary = stats(times);
    eprintln!("API notes_update c={concurrency}: {summary}");
    api.insert(format!("notes_update/c{concurrency}"), summary);
    Ok(())
}

async fn bench_delete_restore(
    app: &axum::Router,
    actor: &str,
    writes: &str,
    ids: &[String],
    offset: usize,
    concurrency: usize,
    api: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut times = Vec::new();
    for round in 0..30 {
        let mut tasks = tokio::task::JoinSet::new();
        for worker in 0..concurrency {
            let app = app.clone();
            let actor = actor.to_owned();
            // Use notes 8..15 here; notes 0..7 are updated by bench_update.
            let path = format!("/api/channels/{writes}/notes/{}", ids[offset + worker]);
            let version = round * 2 + 1;
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                let (status, value) =
                    api_request(&app, "DELETE", &path, &actor, json!({"version":version})).await;
                assert_eq!(status, axum::http::StatusCode::OK, "{value}");
                let (status, value) = api_request(
                    &app,
                    "POST",
                    &format!("{path}/restore"),
                    &actor,
                    Value::Null,
                )
                .await;
                assert_eq!(status, axum::http::StatusCode::OK, "{value}");
                Ok::<_, anyhow::Error>(started.elapsed().as_secs_f64() * 1000.)
            });
        }
        while let Some(value) = tasks.join_next().await {
            times.push(value??);
        }
    }
    let summary = stats(times);
    eprintln!("API notes_delete_restore c={concurrency}: {summary}");
    api.insert(format!("notes_delete_restore/c{concurrency}"), summary);
    Ok(())
}

async fn bench_reorder(
    app: &axum::Router,
    actor: &str,
    active: &str,
    api: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut times = Vec::new();
    for _ in 0..30 {
        let (status, value) = api_request(
            app,
            "GET",
            &format!("/api/channels/{active}/notes"),
            actor,
            Value::Null,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{value}");
        let ids: Vec<String> = value["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| note["id"].as_str().unwrap().to_owned())
            .collect();
        let mut reversed = ids.clone();
        reversed.reverse();
        let started = std::time::Instant::now();
        let (status, value) = api_request(
            app,
            "PATCH",
            &format!("/api/channels/{active}/notes/order"),
            actor,
            json!({"expected_note_ids":ids,"note_ids":reversed}),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{value}");
        times.push(started.elapsed().as_secs_f64() * 1000.);
    }
    let summary = stats(times);
    eprintln!("API notes_reorder c=1: {summary}");
    api.insert("notes_reorder/c1".into(), summary);
    Ok(())
}

async fn bench_cleanup(
    db: toasty::Db,
    concurrency: usize,
    api: &mut serde_json::Map<String, Value>,
) -> anyhow::Result<()> {
    let mut times = Vec::new();
    for _ in 0..30 {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let db = db.clone();
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                backend::routes::channel_notes::purge_expired_notes(&db)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Ok::<_, anyhow::Error>(started.elapsed().as_secs_f64() * 1000.)
            });
        }
        while let Some(value) = tasks.join_next().await {
            times.push(value??);
        }
    }
    let summary = stats(times);
    eprintln!("API notes_cleanup c={concurrency}: {summary}");
    api.insert(format!("notes_cleanup/c{concurrency}"), summary);
    Ok(())
}

#[tokio::test]
#[ignore = "measure the note API on the committed channel_notes_scale_test fixture"]
async fn persisted_channel_notes_api_budget() -> anyhow::Result<()> {
    use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let (client, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(connection);
    let row = client
        .query_one(
            "SELECT id, created_user_id FROM channels WHERE channel_name LIKE 'notes_scale_active_%' LIMIT 1",
            &[],
        )
        .await?;
    let active: String = row.get(0);
    let owner: String = row.get(1);
    let empty: String = client
        .query_one(
            "SELECT id FROM channels WHERE channel_name LIKE 'notes_scale_empty_%' LIMIT 1",
            &[],
        )
        .await?
        .get(0);
    let deleted: String = client
        .query_one(
            "SELECT id FROM channels WHERE channel_name LIKE 'notes_scale_deleted_%' LIMIT 1",
            &[],
        )
        .await?
        .get(0);
    let writes: String = client
        .query_one(
            "SELECT id FROM channels WHERE channel_name LIKE 'notes_scale_writes_%' LIMIT 1",
            &[],
        )
        .await?
        .get(0);
    let empty_list = uuid::Uuid::new_v4().to_string();
    client
        .execute(
            "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false)",
            &[
                &empty_list,
                &format!("notes_scale_listempty_{}", uuid::Uuid::new_v4().simple()),
                &owner,
            ],
        )
        .await?;
    client
        .execute(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
            &[&owner, &empty_list],
        )
        .await?;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    // API-created notes have UUID ids accepted by the note path parser; the
    // fixture's readable ids are only for SQL plan measurements.
    let mut note_ids = Vec::new();
    for _ in 0..16 {
        let (status, value) = api_request(
            &app,
            "POST",
            &format!("/api/channels/{writes}/notes"),
            &owner,
            json!({"title":"seed","content":"seed"}),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{value}");
        note_ids.push(value["data"]["id"].as_str().unwrap().to_owned());
    }
    let mut api = serde_json::Map::new();
    for concurrency in [1, 8] {
        client
            .execute(
                "UPDATE channel_notes SET version=1, deleted_at=NULL \
                 WHERE id IN (SELECT jsonb_array_elements_text($1::text::jsonb))",
                &[&serde_json::json!(note_ids).to_string()],
            )
            .await?;
        bench_static(
            &app,
            &owner,
            "GET",
            &format!("/api/channels/{active}/notes"),
            Value::Null,
            concurrency,
            "notes_list",
            &mut api,
        )
        .await?;
        bench_static(
            &app,
            &owner,
            "GET",
            &format!("/api/channels/{empty_list}/notes"),
            Value::Null,
            concurrency,
            "notes_list_empty",
            &mut api,
        )
        .await?;
        bench_static(
            &app,
            &owner,
            "GET",
            &format!("/api/channels/{deleted}/notes/deleted"),
            Value::Null,
            concurrency,
            "notes_list_deleted",
            &mut api,
        )
        .await?;
        bench_static(
            &app,
            &owner,
            "GET",
            &format!("/api/channels/{writes}/notes/{}", note_ids[0]),
            Value::Null,
            concurrency,
            "notes_show",
            &mut api,
        )
        .await?;
        bench_static(
            &app,
            &owner,
            "POST",
            &format!("/api/channels/{empty}/notes"),
            json!({"title":"api","content":"api"}),
            concurrency,
            "notes_create",
            &mut api,
        )
        .await?;
        bench_update(&app, &owner, &writes, &note_ids, 0, concurrency, &mut api).await?;
        bench_delete_restore(&app, &owner, &writes, &note_ids, 8, concurrency, &mut api).await?;
        bench_cleanup(db.clone(), concurrency, &mut api).await?;
    }
    bench_reorder(&app, &owner, &active, &mut api).await?;
    if let Some(dir) = std::env::var_os("SCALE_PLAN_DIR").map(std::path::PathBuf::from) {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("api.json"), serde_json::to_vec_pretty(&api)?)?;
    }
    Ok(())
}
