use backend::config::Config;
use tokio_postgres::NoTls;

// These fixtures exercise the shared durable queue. Their internal connections
// run concurrently, but fixtures must not drain or lock each other's jobs.
static DISPLAY_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn simultaneous_reactions_do_not_deadlock_on_foreign_key_locks() -> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let at = tokio::spawn(ac);
    let bt = tokio::spawn(bc);
    let id = format!("concurrent-{}", uuid::Uuid::new_v4().simple());
    a.execute(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Concurrent',false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$1,$1,'Concurrent')",
        &[&id],
    )
    .await?;
    // Both inserts' FK checks hold KEY SHARE on the same message. Force that
    // overlap deterministically instead of relying on scheduler timing.
    for client in [&a, &b] {
        client
            .batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
            .await?;
        client
            .query_one("SELECT id FROM messages WHERE id=$1 FOR KEY SHARE", &[&id])
            .await?;
    }
    async fn insert(
        client: &tokio_postgres::Client,
        id: &str,
        emoji: &str,
    ) -> Result<(), tokio_postgres::Error> {
        let result = client.execute("INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES($1||$2,$1,$1,$2)", &[&id,&emoji]).await;
        client
            .batch_execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })
            .await?;
        result.map(|_| ())
    }
    let (first, second) = tokio::join!(insert(&a, &id, "👍"), insert(&b, &id, "🎉"));
    let count: i32 = a
        .query_one(
            "SELECT jsonb_array_length(display_data->'reactions') FROM messages WHERE id=$1",
            &[&id],
        )
        .await?
        .get(0);
    a.execute("DELETE FROM messages WHERE id=$1", &[&id])
        .await?;
    a.execute(
        "UPDATE channels SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1",
        &[&id],
    )
    .await?;
    drop(a);
    drop(b);
    at.await??;
    bt.await??;
    first?;
    second?;
    assert_eq!(count, 2);
    Ok(())
}

#[tokio::test]
async fn display_views_follow_legacy_writes_and_related_updates_atomically() -> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let (client, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let task = tokio::spawn(connection);
    let prefix = format!("display-{}-", uuid::Uuid::new_v4().simple());
    let sql = include_str!("display_views.sql").replace("view-", &prefix);
    let result = client.batch_execute(&sql).await;
    drop(client);
    task.await??;
    result?;
    Ok(())
}

#[tokio::test]
async fn legacy_profile_locks_and_metadata_fanout_do_not_block_other_writers() -> anyhow::Result<()>
{
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    let (observer, oc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(oc);
    let pid: i32 = b.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    b.batch_execute("SET statement_timeout='2s'").await?;
    let id = format!("queue-{}", uuid::Uuid::new_v4().simple());
    a.execute(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO workspace_emojis(name,url,created_by) VALUES($1::text,'/wide.png',$1::text)",
        &[&id],
    )
    .await?;
    // Seed without expensive per-row display projections: exactly the fan-out
    // that made the former metadata phase stop all message writes.
    a.batch_execute("BEGIN; SET LOCAL session_replication_role=replica")
        .await?;
    a.execute("INSERT INTO messages(id,user_id,channel_id,content) SELECT $1||i,$1,$1,':'||$1||':' FROM generate_series(1,20000) i", &[&id]).await?;
    a.batch_execute("COMMIT").await?;
    a.batch_execute("BEGIN; SET LOCAL statement_timeout='2s'")
        .await?;
    // Same order as the backend deployed before this PR: take the user row
    // first. There is no application gate or new function call in this path.
    a.query_one("SELECT id FROM users WHERE id=$1 FOR UPDATE", &[&id])
        .await?;
    a.execute("UPDATE users SET display_name='After' WHERE id=$1", &[&id])
        .await?;
    let write = async {
        b.batch_execute("BEGIN; SET LOCAL statement_timeout='2s'")
            .await?;
        b.execute(
            "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1||'-new',$1,$1,'new')",
            &[&id],
        )
        .await?;
        b.batch_execute("COMMIT").await?;
        Ok::<_, anyhow::Error>(())
    };
    let release = async {
        // Observe the legacy FK lock wait before allowing the profile UPDATE
        // to finish. The former global gate deadlocked in exactly this order.
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let waiting:bool = observer.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')", &[&pid]).await?.get(0);
                if waiting { break Ok::<_,tokio_postgres::Error>(()); }
                tokio::task::yield_now().await;
            }
        }).await??;
        a.execute(
            "UPDATE workspace_emojis SET width=120,height=20 WHERE name=$1",
            &[&id],
        )
        .await?;
        a.batch_execute("COMMIT").await?;
        Ok::<_, anyhow::Error>(())
    };
    let (written, released) = tokio::join!(write, release);
    released?;
    written?;
    // Metadata jobs are still queued. Holding another metadata transaction open
    // must allow writes by the same user to finish without waiting for fan-out.
    a.batch_execute("BEGIN").await?;
    a.execute(
        "UPDATE workspace_emojis SET width=240 WHERE name=$1",
        &[&id],
    )
    .await?;
    b.execute("INSERT INTO messages(id,user_id,channel_id,content) VALUES($1||'-independent',$1,$1,'other')", &[&id]).await?;
    b.execute("INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES($1,$1||'-independent',$1,'👍')", &[&id]).await?;
    a.batch_execute("COMMIT").await?;
    let queued: i64 = a
        .query_one(
            "SELECT count(*) FROM display_refresh_queue WHERE target LIKE $1||'%'",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(queued, 20000);
    // No-op profile SETs do not enqueue work (status-only API updates included).
    a.execute(
        "DELETE FROM display_refresh_queue WHERE target LIKE $1||'%'",
        &[&id],
    )
    .await?;
    a.execute(
        "UPDATE users SET display_name=display_name WHERE id=$1",
        &[&id],
    )
    .await?;
    let count: i64 = a
        .query_one(
            "SELECT count(*) FROM display_refresh_queue WHERE target LIKE $1||'%'",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(count, 0);
    // Only generated rows bypassed source triggers. Remove normal writes with
    // their normal cascades, then bulk-remove just that synthetic workload.
    a.execute(
        "DELETE FROM messages WHERE id IN ($1||'-new',$1||'-independent')",
        &[&id],
    )
    .await?;
    a.batch_execute("BEGIN; SET LOCAL session_replication_role=replica")
        .await?;
    a.execute("DELETE FROM messages WHERE channel_id=$1", &[&id])
        .await?;
    a.batch_execute("COMMIT").await?;
    a.execute("DELETE FROM workspace_emojis WHERE name=$1", &[&id])
        .await?;
    Ok(())
}

#[tokio::test]
async fn queue_skips_busy_rows_and_retains_work_after_rollback() -> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    let id = format!("queue-busy-{}", uuid::Uuid::new_v4().simple());
    a.execute(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$1,$1,'body')",
        &[&id],
    )
    .await?;
    a.execute("UPDATE users SET display_name='After' WHERE id=$1", &[&id])
        .await?;
    a.batch_execute("BEGIN").await?;
    a.query_one("SELECT id FROM messages WHERE id=$1 FOR UPDATE", &[&id])
        .await?;
    b.batch_execute(
        "BEGIN; SET LOCAL statement_timeout='1s'; SELECT refresh_next_display(); COMMIT",
    )
    .await?;
    let exists: bool = b
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM display_refresh_queue WHERE target=$1)",
            &[&id],
        )
        .await?
        .get(0);
    assert!(exists, "busy job must remain durable");
    a.batch_execute("COMMIT").await?;
    // Lock other suites' jobs without deleting them. The second connection
    // skips them and can process only this test's row.
    a.batch_execute("BEGIN").await?;
    a.query(
        "SELECT kind,target FROM display_refresh_queue WHERE target<>$1 FOR UPDATE",
        &[&id],
    )
    .await?;
    b.batch_execute("BEGIN; SELECT refresh_next_display(); ROLLBACK")
        .await?;
    let name: String = a
        .query_one(
            "SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(name, "Before");
    let state =
        backend::state::AppState::new(backend::db::init_db(&config.database_url).await?, config);
    assert!(backend::display_refresh::refresh_one(&state).await?);
    a.batch_execute("COMMIT").await?;
    let name: String = a
        .query_one(
            "SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(name, "After");
    a.execute("DELETE FROM messages WHERE id=$1", &[&id])
        .await?;
    Ok(())
}
