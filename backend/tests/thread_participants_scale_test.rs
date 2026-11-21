//! SCALE_USERS=300000/1000000; use an empty migrated disposable database.
//! Measures `GET /api/threads/{id}`'s participant SQL (`participants_sql`) against a
//! large `users` table. P = participant rows of one thread, L = live (joined, not
//! deactivated, still a channel member) ones. Bound: no Seq Scan on users/thread_participants/
//! channel_user, `thread_participants` rows <= P, `users` and `channel_user` rows <= joined (not
//! left), sort input <= L, logical blocks <= BLOCKS_PER_KEY*joined + 2P + 64.
//! The reply gate SQL (`reply_participation_sql`, run on every thread reply) and the `tracked`-only
//! SQL (`thread_tracked_sql`, the answer without the list) are measured too:
//! no Seq Scan, `thread_participants` rows <= 2, `users` not read, logical blocks <= 64.
//! Each plan is written to `SCALE_PLAN_DIR` before the bounds are checked, so a failing plan survives.
use backend::{
    config::Config,
    routes::{
        messages::reply_participation_sql,
        threads::{participants_sql, thread_tracked_sql},
    },
};
use serde_json::{Value, json};
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

/// Logical blocks per joined key: one `users` probe (index levels + heap) and one `channel_user`
/// probe (index levels), with some room for index levels that grow with the table.
const BLOCKS_PER_KEY: f64 = 10.;

/// The participants SQL with its thread (`$1`) and channel (`$2`) bound as literals.
fn bind_participants(sql: &str, thread: &str, channel: &str) -> String {
    sql.replace("$1", &quoted(thread))
        .replace("$2", &quoted(channel))
}

struct Case {
    name: &'static str,
    thread: String,
    channel: String,
    participants: f64,
    /// Not left: the keys looked up in users. Live = joined minus deactivated.
    joined: f64,
    live: f64,
}

fn work(case: &Case, mode: &str, plan: &Value) -> Value {
    let mut relations = std::collections::BTreeMap::<String, f64>::new();
    let mut max_sort = 0_f64;
    let name = format!("{}/{mode}", case.name);
    walk(&plan["Plan"], &mut |node| {
        let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
        let n = (node["Actual Rows"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
            + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
            * loops;
        if let Some(table) = node["Relation Name"].as_str() {
            *relations.entry(table.into()).or_default() += n;
            assert!(
                !(node["Node Type"] == "Seq Scan"
                    && (table == "users"
                        || table == "thread_participants"
                        || table == "channel_user")),
                "{name}: Seq Scan on {table}"
            );
        }
        if node["Node Type"] == "Sort" {
            let input: f64 = node["Plans"]
                .as_array()
                .map(|children| {
                    children
                        .iter()
                        .map(|c| {
                            c["Actual Rows"].as_f64().unwrap_or(0.)
                                * c["Actual Loops"].as_f64().unwrap_or(0.)
                        })
                        .sum()
                })
                .unwrap_or(0.);
            max_sort = max_sort.max(input);
        }
    });
    let scanned = |table: &str| *relations.get(table).unwrap_or(&0.);
    assert!(
        scanned("thread_participants") <= case.participants,
        "{name}: thread_participants scanned {} > P={}: {relations:?}",
        scanned("thread_participants"),
        case.participants
    );
    assert!(
        scanned("users") <= case.joined,
        "{name}: users scanned {} > joined={}: {relations:?}",
        scanned("users"),
        case.joined
    );
    assert!(
        scanned("channel_user") <= case.joined,
        "{name}: channel_user scanned {} > joined={}: {relations:?}",
        scanned("channel_user"),
        case.joined
    );
    assert!(
        max_sort <= case.live,
        "{name}: sort input {max_sort} > L={}",
        case.live
    );
    assert_eq!(
        plan["Plan"]["Temp Written Blocks"].as_u64().unwrap_or(0),
        0,
        "{name}: disk spill"
    );
    let blocks = plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)
        + plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0);
    let limit = BLOCKS_PER_KEY * case.joined + 2. * case.participants + 64.;
    assert!(
        blocks as f64 <= limit,
        "{name}: {blocks} logical blocks > {limit}"
    );
    json!({"relations":relations,"sort_input":max_sort,"logical_blocks":blocks})
}

/// Work of the reply gate SQL: a fixed number of primary-key probes, whatever P or the totals.
fn gate_work(name: &str, plan: &Value) -> Value {
    let mut relations = std::collections::BTreeMap::<String, f64>::new();
    walk(&plan["Plan"], &mut |node| {
        if let Some(table) = node["Relation Name"].as_str() {
            let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
            let n = (node["Actual Rows"].as_f64().unwrap_or(0.)
                + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
                + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
                * loops;
            *relations.entry(table.into()).or_default() += n;
            assert!(
                node["Node Type"] != "Seq Scan",
                "{name}: Seq Scan on {table}"
            );
        }
    });
    let scanned = |table: &str| *relations.get(table).unwrap_or(&0.);
    assert!(
        scanned("thread_participants") <= 2.,
        "{name}: thread_participants scanned {} > 2: {relations:?}",
        scanned("thread_participants")
    );
    assert!(
        scanned("users") == 0.,
        "{name}: users scanned {} > 0: {relations:?}",
        scanned("users")
    );
    let blocks = plan["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)
        + plan["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0);
    assert!(blocks <= 64, "{name}: {blocks} logical blocks > 64");
    json!({"relations":relations,"sort_input":0,"logical_blocks":blocks})
}

/// Writes a plan before its bounds are checked, so the plan of a failing case stays available.
fn save_plan(dir: &Option<std::path::PathBuf>, file: &str, plan: &Value) -> anyhow::Result<()> {
    if let Some(dir) = dir {
        std::fs::write(dir.join(file), serde_json::to_vec_pretty(plan)?)?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "inserts 300k/1m users"]
async fn participants_sql_does_not_scan_users() -> anyhow::Result<()> {
    let users: i32 = std::env::var("SCALE_USERS")
        .unwrap_or_else(|_| "300000".into())
        .parse()?;
    assert!(users >= 300000);
    let url = Config::from_env().database_url;
    let (mut client, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    client
        .batch_execute("SET jit=off; SET statement_timeout='300s'")
        .await?;
    let tag = uuid::Uuid::new_v4().to_string();
    let channel = format!("{tag}-channel");
    let owner = format!("{tag}-owner");
    // (name, participants P, left, deactivated, no longer channel members).
    // Live = P - left - deactivated - non-members.
    let specs: [(&str, i32, i32, i32, i32); 5] = [
        ("small", 50, 0, 0, 0),
        ("large", 10_000, 500, 500, 500),
        ("empty", 0, 0, 0, 0),
        ("all_left", 100, 100, 0, 0),
        // Mostly left rows: the range scan reads them all, the `users` probes only the 500 joined.
        ("many_left", 10_000, 9_500, 0, 0),
    ];
    let tx = client.transaction().await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&owner],
    )
    .await?;
    tx.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)", &[&channel, &owner]).await?;
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    tx.execute("INSERT INTO users(id,user_name,display_name) SELECT $1||'-u-'||lpad(n::text,9,'0'),$1||'-u-'||lpad(n::text,9,'0'),'U' FROM generate_series(1,$2::integer)n", &[&tag, &users]).await?;
    let mut cases = Vec::new();
    for (name, p, left, deactivated, non_members) in specs {
        let thread = format!("{tag}-thread-{name}");
        tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'root','2025-01-01')", &[&thread, &owner, &channel]).await?;
        // Spread participants over the whole id space (37 is coprime with 300k/1m), so
        // index leaves and heap pages are not shared more than in production.
        // Rows 1..=left are left; the next `deactivated` users are deactivated.
        // `many_left` uses other users than the cases above, whose fixtures deactivate or remove some.
        let user_offset: i32 = if name == "many_left" { 100_000 } else { 0 };
        tx.execute("INSERT INTO thread_participants(thread_id,user_id,joined_at,left_at) SELECT $1,$2||'-u-'||lpad((1+((n+$6::integer)*37)%$3::integer)::text,9,'0'),timestamp '2025-01-01'+n*interval '1 second',CASE WHEN n<=$5::integer THEN timestamp '2025-02-01' END FROM generate_series(1,$4::integer)n", &[&thread, &tag, &users, &p, &left, &user_offset]).await?;
        if deactivated > 0 {
            tx.execute("UPDATE users SET is_deactivated=true WHERE id IN (SELECT user_id FROM thread_participants WHERE thread_id=$1 AND left_at IS NULL ORDER BY joined_at DESC LIMIT $2::integer)", &[&thread, &deactivated]).await?;
        }
        cases.push(Case {
            name,
            thread,
            channel: channel.clone(),
            participants: f64::from(p),
            joined: f64::from(p - left),
            live: f64::from(p - left - deactivated - non_members),
        });
    }
    // Every participant is a member of the channel, except the earliest `non_members` live ones of
    // a case (they left the channel but kept the thread row). Those users are in no other case.
    tx.execute("INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT DISTINCT user_id,$2,now() FROM thread_participants WHERE thread_id LIKE $1||'-thread-%' ON CONFLICT DO NOTHING", &[&tag, &channel]).await?;
    for (case, (_, _, _, _, non_members)) in cases.iter().zip(specs) {
        if non_members > 0 {
            tx.execute("DELETE FROM channel_user WHERE channel_id=$2 AND user_id IN (SELECT user_id FROM thread_participants WHERE thread_id=$1 AND left_at IS NULL ORDER BY joined_at LIMIT $3::integer)", &[&case.thread, &channel, &non_members]).await?;
        }
    }
    // Unrelated history: `users` rows spread over users/30 other threads (30 rows each),
    // so the planner sees many distinct thread_id values, as in production.
    tx.execute("INSERT INTO thread_participants(thread_id,user_id) SELECT $1||'-noise-'||(n%($2::integer/30)),$1||'-u-'||lpad((1+(n*37)%$2::integer)::text,9,'0') FROM generate_series(1,$2::integer)n", &[&tag, &users]).await?;
    tx.batch_execute("SET LOCAL session_replication_role=origin")
        .await?;
    tx.commit().await?;
    client
        .batch_execute("ANALYZE users; ANALYZE thread_participants; ANALYZE messages")
        .await?;
    // The membership probe is an index-only scan; a vacuumed table (as in production) has no heap
    // fetches. VACUUM cannot run inside a multi-statement batch.
    client
        .batch_execute("VACUUM (ANALYZE) channel_user")
        .await?;

    let output = std::env::var_os("SCALE_PLAN_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir)?;
    }
    let sql = participants_sql();
    let mut metrics = serde_json::Map::new();
    for case in &cases {
        // Result correctness: tracked / live count / first 50.
        let result = client
            .simple_query(&bind_participants(sql, &case.thread, &case.channel))
            .await?;
        let value: Value = serde_json::from_str(rows(&result)[0])?;
        assert_eq!(value["tracked"], case.participants > 0., "{}", case.name);
        assert_eq!(value["count"].as_f64(), Some(case.live), "{}", case.name);
        assert_eq!(
            value["items"].as_array().unwrap().len() as f64,
            case.live.min(50.),
            "{}",
            case.name
        );

        // Literal (custom plan), then generic plan; each run twice (first / rerun).
        for run in ["custom_first", "custom_rerun"] {
            let raw = client
                .simple_query(&format!(
                    "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {}",
                    bind_participants(sql, &case.thread, &case.channel)
                ))
                .await?;
            let plan: Value = serde_json::from_str(rows(&raw)[0])?;
            save_plan(&output, &format!("{}-{run}.json", case.name), &plan)?;
            let measured = work(case, run, &plan[0]);
            eprintln!(
                "{users} {}/{run} work={measured} planning={} execution={}",
                case.name, plan[0]["Planning Time"], plan[0]["Execution Time"]
            );
            metrics.insert(format!("{}/{run}", case.name), measured);
        }
        client.batch_execute(&format!("DEALLOCATE ALL; PREPARE participants(text,text) AS {sql}; SET plan_cache_mode=force_generic_plan")).await?;
        for run in ["generic_first", "generic_rerun"] {
            let raw = client
                .simple_query(&format!(
                    "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) EXECUTE participants({},{})",
                    quoted(&case.thread),
                    quoted(&case.channel)
                ))
                .await?;
            let plan: Value = serde_json::from_str(rows(&raw)[0])?;
            save_plan(&output, &format!("{}-{run}.json", case.name), &plan)?;
            let measured = work(case, run, &plan[0]);
            eprintln!(
                "{users} {}/{run} work={measured} planning={} execution={}",
                case.name, plan[0]["Planning Time"], plan[0]["Execution Time"]
            );
            metrics.insert(format!("{}/{run}", case.name), measured);
        }
        client
            .batch_execute("DEALLOCATE ALL; RESET plan_cache_mode")
            .await?;

        // Reply gate for a user who is not in the thread (the owner): `tracked` must come from
        // one index entry of this thread, not a scan that skips other threads' rows.
        let gate = reply_participation_sql();
        let custom = gate
            .replace("$1", &quoted(&case.thread))
            .replace("$2", &quoted(&owner));
        let raw = client
            .simple_query(&format!(
                "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {custom}"
            ))
            .await?;
        let plan: Value = serde_json::from_str(rows(&raw)[0])?;
        save_plan(&output, &format!("{}-gate_custom.json", case.name), &plan)?;
        metrics.insert(
            format!("{}/gate_custom", case.name),
            gate_work(&format!("{}/gate_custom", case.name), &plan[0]),
        );
        client
            .batch_execute(&format!(
                "PREPARE gate(text,text) AS {gate}; SET plan_cache_mode=force_generic_plan"
            ))
            .await?;
        let raw = client
            .simple_query(&format!(
                "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) EXECUTE gate({},{})",
                quoted(&case.thread),
                quoted(&owner)
            ))
            .await?;
        let plan: Value = serde_json::from_str(rows(&raw)[0])?;
        save_plan(&output, &format!("{}-gate_generic.json", case.name), &plan)?;
        metrics.insert(
            format!("{}/gate_generic", case.name),
            gate_work(&format!("{}/gate_generic", case.name), &plan[0]),
        );
        client
            .batch_execute("DEALLOCATE ALL; RESET plan_cache_mode")
            .await?;

        // `tracked` alone (the answer without the participant list, `?participants=none`).
        let tracked = thread_tracked_sql();
        let raw = client
            .simple_query(&format!(
                "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {}",
                tracked.replace("$1", &quoted(&case.thread))
            ))
            .await?;
        let plan: Value = serde_json::from_str(rows(&raw)[0])?;
        save_plan(
            &output,
            &format!("{}-tracked_custom.json", case.name),
            &plan,
        )?;
        metrics.insert(
            format!("{}/tracked_custom", case.name),
            gate_work(&format!("{}/tracked_custom", case.name), &plan[0]),
        );
        client
            .batch_execute(&format!(
                "PREPARE tracked(text) AS {tracked}; SET plan_cache_mode=force_generic_plan"
            ))
            .await?;
        let raw = client
            .simple_query(&format!(
                "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) EXECUTE tracked({})",
                quoted(&case.thread)
            ))
            .await?;
        let plan: Value = serde_json::from_str(rows(&raw)[0])?;
        save_plan(
            &output,
            &format!("{}-tracked_generic.json", case.name),
            &plan,
        )?;
        metrics.insert(
            format!("{}/tracked_generic", case.name),
            gate_work(&format!("{}/tracked_generic", case.name), &plan[0]),
        );
        client
            .batch_execute("DEALLOCATE ALL; RESET plan_cache_mode")
            .await?;
    }

    // Single 30 runs and 8 concurrent x 30 runs on the large thread. Times are reported only.
    let large_case = cases.iter().find(|c| c.name == "large").unwrap();
    let large_statement = bind_participants(sql, &large_case.thread, &large_case.channel);
    let timings = |mut samples: Vec<f64>| {
        samples.sort_by(|a, b| a.total_cmp(b));
        json!({"p50":samples[samples.len()/2],"p95":samples[samples.len()*95/100],"max":samples[samples.len()-1]})
    };
    let mut single = Vec::new();
    for _ in 0..30 {
        let started = std::time::Instant::now();
        client.simple_query(&large_statement).await?;
        single.push(started.elapsed().as_secs_f64() * 1000.);
    }
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let url = url.clone();
        let statement = large_statement.clone();
        tasks.push(tokio::spawn(async move {
            let (client, connection) = tokio_postgres::connect(&url, NoTls).await?;
            tokio::spawn(connection);
            let mut samples = Vec::new();
            for _ in 0..30 {
                let started = std::time::Instant::now();
                client.simple_query(&statement).await?;
                samples.push(started.elapsed().as_secs_f64() * 1000.);
            }
            anyhow::Ok(samples)
        }));
    }
    let mut concurrent = Vec::new();
    for task in tasks {
        concurrent.extend(task.await??);
    }
    eprintln!(
        "{users} large single_ms={} concurrent8_ms={}",
        timings(single.clone()),
        timings(concurrent.clone())
    );
    metrics.insert(
        "timing_ms".into(),
        json!({"single":timings(single),"concurrent8":timings(concurrent)}),
    );
    if let Some(dir) = &output {
        std::fs::write(dir.join("work.json"), serde_json::to_vec_pretty(&metrics)?)?;
    }
    Ok(())
}
