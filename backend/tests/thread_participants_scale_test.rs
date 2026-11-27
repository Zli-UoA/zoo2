//! SCALE_USERS=300000/1000000; use an empty migrated disposable database.
//! `SCALE_HISTORY` (default: SCALE_USERS) is the number of unrelated `thread_participants` rows.
//! Setting it apart from SCALE_USERS separates the two: a fixed `users` table with a growing
//! history (the work must not grow), and a growing `users` table with a fixed history (only the
//! index depth may).
//! Measures `GET /api/threads/{id}`'s participant SQL (`participants_sql`) against a
//! large `users` table. P = participant rows of one thread, L = live (joined, not
//! deactivated, still a channel member) ones. Bound: no Seq Scan on users/thread_participants/
//! channel_user, `thread_participants` rows <= P, `users` and `channel_user` rows <= joined (not
//! left), sort input <= L, logical blocks <= BLOCKS_PER_KEY*joined + 2P + 64.
//! The reply gate SQL (`reply_participation_sql`, run on every thread reply) and the `tracked`-only
//! SQL (`thread_tracked_sql`, the answer without the list) are measured too:
//! no Seq Scan, `thread_participants` rows <= 2, `users` not read, logical blocks <= 64.
//! Each plan is written to `SCALE_PLAN_DIR` before the bounds are checked, so a failing plan survives.
//! When the connected server is a postgres process of this host (its backend's working directory is the
//! server's `data_directory`), the CPU time (user + system, `/proc/<pid>/stat`) of 30 single runs on the
//! large thread is reported too (`cpu_ms_per_call`); otherwise (a container, another host, a role that
//! cannot `SHOW data_directory`) it is `null`.
use backend::{
    config::Config,
    routes::{
        messages::reply_participation_sql,
        threads::{participants_sql, thread_tracked_sql},
    },
};
#[path = "support/spread.rs"]
mod spread;
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
/// CPU time (user + system, in ms) of the server backend `pid`, from `/proc/<pid>/stat`. `None`
/// unless that process is this server's: a pid alone does not say so (another host or pid
/// namespace has its own, possibly equal, pids), so the process must be a postgres whose working
/// directory is the server's `data_directory` (a backend runs there). A container, another host or
/// a role that cannot read `data_directory` gets `None`.
fn backend_cpu_ms(pid: i32, data_directory: Option<&str>) -> Option<f64> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    if !name.trim().starts_with("postgres") {
        return None;
    }
    let cwd = std::fs::canonicalize(std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?).ok()?;
    if cwd != std::fs::canonicalize(data_directory?).ok()? {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may hold spaces: the fields after the last `)` start at the state (field 3),
    // so utime (field 14) and stime (field 15) are the 12th and 13th.
    let fields: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    let per_second: f64 = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(100.);
    Some(ticks / per_second * 1000.)
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
    let history: i32 = std::env::var("SCALE_HISTORY")
        .map(|value| value.parse())
        .unwrap_or(Ok(users))?;
    assert!(history >= 300000);
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
    let stride = spread::stride(users);
    let mut cases = Vec::new();
    for (name, p, left, deactivated, non_members) in specs {
        let thread = format!("{tag}-thread-{name}");
        tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'root','2025-01-01')", &[&thread, &owner, &channel]).await?;
        // Spread participants over the whole id space (`spread::stride`), so
        // index leaves and heap pages are not shared more than in production.
        // Rows 1..=left are left; the next `deactivated` users are deactivated.
        // `many_left` uses other users than the cases above, whose fixtures deactivate or remove some.
        let user_offset: i32 = if name == "many_left" { 100_000 } else { 0 };
        tx.execute("INSERT INTO thread_participants(thread_id,user_id,joined_at,left_at) SELECT $1,$2||'-u-'||lpad((1+((n+$6::integer)::bigint*$7::bigint)%($3::integer)::bigint)::text,9,'0'),timestamp '2025-01-01'+n*interval '1 second',CASE WHEN n<=$5::integer THEN timestamp '2025-02-01' END FROM generate_series(1,$4::integer)n", &[&thread, &tag, &users, &p, &left, &user_offset, &stride]).await?;
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
    // Unrelated history: `history` rows spread over history/30 other threads (30 rows each),
    // so the planner sees many distinct thread_id values, as in production.
    if history == users {
        tx.execute("INSERT INTO thread_participants(thread_id,user_id) SELECT $1||'-noise-'||(n%($2::integer/30)),$1||'-u-'||lpad((1+(n*37)%$2::integer)::text,9,'0') FROM generate_series(1,$2::integer)n", &[&tag, &users]).await?;
    } else {
        // Row n is the (n/T)-th user of thread n%T, T = history/30: the 30 users of one thread
        // differ, so the primary key holds for any history. (The noise rows are never probed; only the
        // thread_participants index grows with them.)
        tx.execute("INSERT INTO thread_participants(thread_id,user_id) SELECT $1||'-noise-'||(n%($2::integer/30)),$1||'-u-'||lpad((1+((n%($2::integer/30))*37+(n/($2::integer/30))*1013)%$3::integer)::text,9,'0') FROM generate_series(1,$2::integer)n", &[&tag, &history, &users]).await?;
    }
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
                "U={users} H={history} {}/{run} work={measured} planning={} execution={}",
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
                "U={users} H={history} {}/{run} work={measured} planning={} execution={}",
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
    // The CPU time of the backend that runs the 30 statements, to tell the work from the waiting.
    let pid: i32 = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let data_directory: Option<String> = match client.query_one("SHOW data_directory", &[]).await {
        Ok(row) => Some(row.get(0)),
        Err(_) => None, // Not a superuser or `pg_read_all_settings` member: no CPU time.
    };
    let cpu_before = backend_cpu_ms(pid, data_directory.as_deref());
    for _ in 0..30 {
        let started = std::time::Instant::now();
        client.simple_query(&large_statement).await?;
        single.push(started.elapsed().as_secs_f64() * 1000.);
    }
    let cpu_per_call = cpu_before
        .zip(backend_cpu_ms(pid, data_directory.as_deref()))
        .map(|(before, after)| (after - before) / 30.);
    eprintln!("U={users} H={history} large single backend cpu_ms_per_call={cpu_per_call:?}");
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
        "U={users} H={history} large single_ms={} concurrent8_ms={}",
        timings(single.clone()),
        timings(concurrent.clone())
    );
    metrics.insert(
        "timing_ms".into(),
        json!({"single":timings(single),"concurrent8":timings(concurrent),"cpu_ms_per_call":cpu_per_call}),
    );
    if let Some(dir) = &output {
        std::fs::write(dir.join("work.json"), serde_json::to_vec_pretty(&metrics)?)?;
    }
    Ok(())
}

/// #620 のスレッド返信通知の参加者SQL。P = そのスレッドの参加行、L = 未退出の参加行。
/// 探索上限: `thread_participants` 走査 <= P、`channel_user` 主キー参照 <= L、
/// `users` 走査 0、マージ結合のソート入力 <= L（ディスク退避なし）、
/// 論理ブロック <= 2P + BLOCKS_PER_KEY*L + 1024（索引の段数と計画の差の余裕）。
/// 無関係な参加行 H を10倍にしても探索量が増えないことを、30万/100万件で比較する。
#[tokio::test]
#[ignore = "inserts 300k/1m users"]
async fn notification_participants_sql_does_not_scan_users() -> anyhow::Result<()> {
    use backend::notifications::notification_participants_sql;

    let users: i32 = std::env::var("SCALE_USERS")
        .unwrap_or_else(|_| "300000".into())
        .parse()?;
    assert!(users >= 300000);
    let history: i32 = std::env::var("SCALE_HISTORY")
        .map(|v| v.parse())
        .unwrap_or(Ok(users))?;
    assert!(history >= 300000);
    let url = Config::from_env().database_url;
    let (mut client, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    client
        .batch_execute("SET jit=off; SET statement_timeout='300s'")
        .await?;
    let tag = uuid::Uuid::new_v4().to_string();
    let channel = format!("{tag}-channel");
    let noise_channel = format!("{tag}-noise-channel");
    let owner = format!("{tag}-owner");
    let thread = format!("{tag}-thread");
    let (p, left) = (10_000_i32, 500_i32);
    let live = p - left;
    let tx = client.transaction().await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&owner],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&channel, &owner],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'root','2025-01-01')",
        &[&thread, &owner, &channel],
    )
    .await?;
    tx.batch_execute("SET LOCAL session_replication_role=replica")
        .await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) SELECT $1||'-u-'||lpad(n::text,9,'0'),$1||'-u-'||lpad(n::text,9,'0'),'U' FROM generate_series(1,$2::integer)n",
        &[&tag, &users],
    )
    .await?;
    let stride = spread::stride(users);
    tx.execute(
        "INSERT INTO thread_participants(thread_id,user_id,joined_at,left_at) \
         SELECT $1,$2||'-u-'||lpad((1+(n::bigint*$5::bigint)%($3::integer)::bigint)::text,9,'0'), \
                timestamp '2025-01-01'+n*interval '1 second', \
                CASE WHEN n<=$4::integer THEN timestamp '2025-02-01' END \
         FROM generate_series(1,$6::integer)n",
        &[&thread, &tag, &users, &left, &stride, &p],
    )
    .await?;
    // 半分は all、半分は mute。mute でも参加行の走査は同じで、レベルだけが返る。
    tx.execute(
        "INSERT INTO channel_user(user_id,channel_id,joined_at,notification_level) \
         SELECT user_id,$2,now(),CASE WHEN row_number() OVER (ORDER BY user_id)%2=0 \
                THEN 'mute' ELSE 'all' END \
         FROM thread_participants WHERE thread_id=$1 AND left_at IS NULL",
        &[&thread, &channel],
    )
    .await?;
    let threads = history / 30;
    let history_rows = threads * 30;
    tx.execute(
        "INSERT INTO thread_participants(thread_id,user_id) \
         SELECT $1||'-noise-'||(n%($2::integer/30)), \
                $1||'-u-'||lpad((1+((n%($2::integer/30))*37+(n/($2::integer/30))*1013)%$3::integer)::text,9,'0') \
         FROM generate_series(1,$4::integer)n",
        &[&tag, &history, &users, &history_rows],
    )
    .await?;
    // 実際の channel_user は全ユーザー分ある。小さなdummy表だとプランナが Seq Scan を選ぶため、
    // 無関係なメンバーシップも置いて主キー参照を使う計画を検証する。
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&noise_channel, &owner],
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) \
         SELECT $1||'-u-'||lpad(n::text,9,'0'),$2::text,now() FROM generate_series(1,$3::integer)n \
         ON CONFLICT DO NOTHING",
        &[&tag, &noise_channel, &users],
    )
    .await?;
    tx.batch_execute("SET LOCAL session_replication_role=origin")
        .await?;
    tx.commit().await?;
    client
        .batch_execute("ANALYZE users; ANALYZE thread_participants")
        .await?;
    client
        .batch_execute("VACUUM (ANALYZE) channel_user")
        .await?;

    let output = std::env::var_os("SCALE_PLAN_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &output {
        std::fs::create_dir_all(dir)?;
    }
    let sql = notification_participants_sql();
    let literal = sql
        .replace("$1", &quoted(&thread))
        .replace("$2", &quoted(&channel));
    // 結果の確認: 未退出 L 行だけが返り、レベルが all / mute で埋まっている。
    let result = client.simple_query(&literal).await?;
    let values: Vec<Value> = rows(&result)
        .into_iter()
        .map(|row| serde_json::from_str(row))
        .collect::<Result<_, _>>()?;
    assert_eq!(values.len() as i32, live);
    assert!(
        values
            .iter()
            .all(|v| v["level"] == "all" || v["level"] == "mute")
    );

    for (mode, statement) in [
        ("custom", literal.clone()),
        (
            "generic",
            format!(
                "EXECUTE notification_participants({},{})",
                quoted(&thread),
                quoted(&channel)
            ),
        ),
    ] {
        if mode == "generic" {
            client
                .batch_execute(&format!("PREPARE notification_participants(text,text) AS {sql}; SET plan_cache_mode=force_generic_plan"))
                .await?;
        }
        for run in ["first", "rerun"] {
            let raw = client
                .simple_query(&format!(
                    "EXPLAIN(ANALYZE,BUFFERS,TIMING OFF,FORMAT JSON) {statement}"
                ))
                .await?;
            let plan: Value = serde_json::from_str(rows(&raw)[0])?;
            save_plan(&output, &format!("notification-{mode}-{run}.json"), &plan)?;
            let mut relations = std::collections::BTreeMap::<String, f64>::new();
            let mut max_loops = 0_f64;
            let mut sort_input = 0_f64;
            let mut external_sort = false;
            let mut blocks = 0_f64;
            walk(&plan[0]["Plan"], &mut |node| {
                let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
                max_loops = max_loops.max(loops);
                let n = (node["Actual Rows"].as_f64().unwrap_or(0.)
                    + node["Rows Removed by Filter"].as_f64().unwrap_or(0.))
                    * loops;
                if let Some(table) = node["Relation Name"].as_str() {
                    *relations.entry(table.into()).or_default() += n;
                    // Gather / Join ノードは子の合計を持つので、表を読むノードだけで数える。
                    blocks += node["Shared Hit Blocks"].as_f64().unwrap_or(0.)
                        + node["Shared Read Blocks"].as_f64().unwrap_or(0.);
                    assert!(
                        !(node["Node Type"] == "Seq Scan"
                            && (table == "users"
                                || table == "thread_participants"
                                || table == "channel_user")),
                        "notification {mode}/{run}: Seq Scan on {table}"
                    );
                }
                if node["Node Type"] == "Sort" {
                    sort_input += node["Actual Rows"].as_f64().unwrap_or(0.) * loops;
                    external_sort |= node["Sort Method"] == "external merge";
                    blocks += node["Shared Hit Blocks"].as_f64().unwrap_or(0.)
                        + node["Shared Read Blocks"].as_f64().unwrap_or(0.);
                }
            });
            let scanned_participants = relations.get("thread_participants").copied().unwrap_or(0.);
            let probed_members = relations.get("channel_user").copied().unwrap_or(0.);
            let read_users = relations.get("users").copied().unwrap_or(0.);
            // 並列走査では worker ごとの端数が出るため 64 行の余裕を置く。
            assert!(
                scanned_participants <= f64::from(p) + 64.,
                "{mode}/{run}: {scanned_participants}"
            );
            assert!(
                probed_members <= f64::from(live),
                "{mode}/{run}: {probed_members}"
            );
            assert_eq!(read_users, 0., "{mode}/{run}: users must not be read");
            assert!(
                sort_input <= f64::from(live),
                "{mode}/{run}: sort input {sort_input}"
            );
            assert!(!external_sort, "{mode}/{run}: external sort");
            assert!(
                max_loops <= f64::from(live) + 1.,
                "{mode}/{run}: loops={max_loops}"
            );
            assert!(
                blocks <= 2. * f64::from(p) + BLOCKS_PER_KEY * f64::from(live) + 1024.,
                "{mode}/{run}: blocks={blocks}"
            );
            eprintln!(
                "U={users} H={history} notification/{mode}/{run} participants={scanned_participants} channel_user={probed_members} users={read_users} blocks={blocks} loops={max_loops} execution={}ms",
                plan[0]["Execution Time"]
            );
        }
    }
    client
        .batch_execute("DEALLOCATE ALL; RESET plan_cache_mode")
        .await?;

    // 単独30回と8同時30回（初回を捨てず、SQL往復と接続待ちを含む）。
    let timings = |mut samples: Vec<f64>| {
        samples.sort_by(|a, b| a.total_cmp(b));
        json!({"p50":samples[samples.len()/2],"p95":samples[samples.len()*95/100],"max":samples[samples.len()-1]})
    };
    let mut single = Vec::new();
    for _ in 0..30 {
        let started = std::time::Instant::now();
        client.simple_query(&literal).await?;
        single.push(started.elapsed().as_secs_f64() * 1000.);
    }
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let url = url.clone();
        let statement = literal.clone();
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
        "U={users} H={history} notification single_ms={} concurrent8_ms={}",
        timings(single.clone()),
        timings(concurrent.clone())
    );
    if let Some(dir) = &output {
        std::fs::write(
            dir.join("notification-work.json"),
            serde_json::to_vec_pretty(&json!({
                "single": timings(single), "concurrent8": timings(concurrent)
            }))?,
        )?;
    }
    Ok(())
}
