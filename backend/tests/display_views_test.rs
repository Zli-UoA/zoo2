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
    let _guard = DISPLAY_TESTS.lock().await;
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
    // Metadata records small scopes; it does not materialize 20,000 row jobs
    // inside the profile transaction. Source writes reconcile independently.
    assert!((1..10).contains(&queued), "bounded source queue: {queued}");
    let scopes: i64 = a
        .query_one(
            "SELECT count(*) FROM display_refresh_queue WHERE kind='user_messages' AND target=$1",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(scopes, 1);
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

async fn queue_client(url: &str) -> anyhow::Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(connection);
    Ok(client)
}

async fn isolate_queue(client: &tokio_postgres::Client, prefix: &str) -> anyhow::Result<()> {
    client.batch_execute("BEGIN").await?;
    client.query("SELECT kind,target FROM display_refresh_queue WHERE NOT starts_with(target,$1) FOR UPDATE", &[&prefix]).await?;
    Ok(())
}

async fn drain_queue(client: &tokio_postgres::Client) -> anyhow::Result<()> {
    for _ in 0..1000 {
        let expanded: bool = client
            .query_one("SELECT expand_next_display_scope()", &[])
            .await?
            .get(0);
        let refreshed: bool = client
            .query_one("SELECT refresh_next_display()", &[])
            .await?
            .get(0);
        if !expanded && !refreshed {
            return Ok(());
        }
    }
    anyhow::bail!("Fixture queue did not converge")
}

#[tokio::test]
async fn overlapping_source_and_metadata_commits_converge_in_both_orders() -> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let source = queue_client(&config.database_url).await?;
    let metadata = queue_client(&config.database_url).await?;
    let worker = queue_client(&config.database_url).await?;
    let others = queue_client(&config.database_url).await?;
    for metadata_commits_first in [true, false] {
        let id = format!("race-{}", uuid::Uuid::new_v4().simple());
        let author = format!("{id}-author");
        let recipient = format!("{id}-recipient");
        let parent = format!("{id}-parent");
        let reply = format!("{id}-reply");
        let emoji = format!("{id}-emoji");
        let alias = format!("{id}-alias");
        source.execute("INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false),($2::text,$2::text,'Recipient',false)", &[&author,&recipient]).await?;
        source.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)", &[&id,&recipient]).await?;
        source
            .execute(
                "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'Parent')",
                &[&parent, &recipient, &id],
            )
            .await?;
        isolate_queue(&others, &id).await?;
        drain_queue(&worker).await?;
        source
            .batch_execute("BEGIN; SET LOCAL statement_timeout='3s'")
            .await?;
        metadata
            .batch_execute("BEGIN; SET LOCAL statement_timeout='3s'")
            .await?;
        if !metadata_commits_first {
            // The metadata trigger runs before the uncommitted source exists.
            // Its commit will happen only after the source job is consumed.
            metadata
                .execute(
                    "UPDATE users SET display_name='After' WHERE id=$1",
                    &[&author],
                )
                .await?;
            metadata.execute("INSERT INTO workspace_emojis(name,url,aliases,width,height,created_by) VALUES($1,'/race.png',jsonb_build_array($2::text),120,20,$3)", &[&emoji,&alias,&recipient]).await?;
        }
        source.execute("INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES($1,$2,$3,':'||$4::text||':',$5)", &[&reply,&author,&id,&alias,&parent]).await?;
        source.execute("INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES($1,$2,$3,':'||$4::text||':')", &[&id,&parent,&author,&alias]).await?;
        source.execute("INSERT INTO notification_events(id,user_id,actor_id,channel_id,message_id,notification_type,source) VALUES($1,$2,$3,$1,$4,'mention','channel')", &[&id,&recipient,&author,&reply]).await?;
        if metadata_commits_first {
            // Reproduce the reported order: a post has already cached old
            // metadata, but remains invisible to the metadata transaction.
            metadata
                .execute(
                    "UPDATE users SET display_name='After' WHERE id=$1",
                    &[&author],
                )
                .await?;
            metadata.execute("INSERT INTO workspace_emojis(name,url,aliases,width,height,created_by) VALUES($1,'/race.png',jsonb_build_array($2::text),120,20,$3)", &[&emoji,&alias,&recipient]).await?;
            metadata.batch_execute("COMMIT").await?;
            // Consuming a scope before the post commits must still be safe.
            drain_queue(&worker).await?;
            source.batch_execute("COMMIT").await?;
        } else {
            source.batch_execute("COMMIT").await?;
            drain_queue(&worker).await?;
            let before: String = worker
                .query_one(
                    "SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1",
                    &[&reply],
                )
                .await?
                .get(0);
            assert_eq!(before, "Before");
            metadata.batch_execute("COMMIT").await?;
        }
        drain_queue(&worker).await?;
        let drift:i64=worker.query_one("SELECT count(*) FROM messages m JOIN message_display v USING(id) WHERE m.channel_id=$1 AND m.display_data IS DISTINCT FROM v.display_data", &[&id]).await?.get(0);
        assert_eq!(drift, 0, "metadata_commits_first={metadata_commits_first}");
        let row=worker.query_one("SELECT display_data->'author'->>'display_name',display_data->'emojis'->$2->>'width' FROM messages WHERE id=$1", &[&reply,&alias]).await?;
        assert_eq!(row.get::<_, String>(0), "After");
        assert_eq!(row.get::<_, String>(1), "120");
        let drift:i64=worker.query_one("SELECT count(*) FROM notification_events n JOIN notification_display v USING(id) WHERE n.id=$1 AND n.display_data IS DISTINCT FROM v.display_data", &[&id]).await?.get(0);
        assert_eq!(
            drift, 0,
            "notification metadata_commits_first={metadata_commits_first}"
        );
        source
            .execute("DELETE FROM messages WHERE channel_id=$1", &[&id])
            .await?;
        source
            .execute("DELETE FROM workspace_emojis WHERE name=$1", &[&emoji])
            .await?;
        others.batch_execute("COMMIT").await?;
    }
    Ok(())
}

#[tokio::test]
async fn deferred_metadata_expands_in_bounded_batches_without_repainting_sources()
-> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let client = queue_client(&config.database_url).await?;
    let others = queue_client(&config.database_url).await?;
    let id = format!("batch-{}", uuid::Uuid::new_v4().simple());
    client.execute("INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false)", &[&id]).await?;
    client.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)", &[&id]).await?;
    client
        .batch_execute("BEGIN; SET LOCAL session_replication_role=replica")
        .await?;
    client.execute("INSERT INTO messages(id,user_id,channel_id,content) SELECT $1||i,$1,$1,'batch' FROM generate_series(1,145) i", &[&id]).await?;
    client.batch_execute("COMMIT").await?;
    client
        .execute("UPDATE users SET display_name='After' WHERE id=$1", &[&id])
        .await?;
    isolate_queue(&others, &id).await?;
    let before:i64=client.query_one("SELECT count(*) FROM display_refresh_queue WHERE kind='message' AND starts_with(target,$1)", &[&id]).await?.get(0);
    assert_eq!(before, 0, "source trigger records only scopes");
    client
        .query_one("SELECT expand_next_display_scope()", &[])
        .await?;
    let first:i64=client.query_one("SELECT count(*) FROM display_refresh_queue WHERE kind='message' AND starts_with(target,$1)", &[&id]).await?.get(0);
    assert_eq!(first, 128, "scope expansion is bounded per transaction");
    let painted: i64 = client
        .query_one(
            "SELECT count(*) FROM messages WHERE channel_id=$1 AND display_data<>'{}'::jsonb",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(
        painted, 0,
        "scope expansion does not lock or repaint messages"
    );
    drain_queue(&client).await?;
    let drift:i64=client.query_one("SELECT count(*) FROM messages m JOIN message_display v USING(id) WHERE m.channel_id=$1 AND m.display_data IS DISTINCT FROM v.display_data", &[&id]).await?.get(0);
    assert_eq!(drift, 0);
    client
        .batch_execute("BEGIN; SET LOCAL session_replication_role=replica")
        .await?;
    client
        .execute("DELETE FROM messages WHERE channel_id=$1", &[&id])
        .await?;
    client.batch_execute("COMMIT").await?;
    others.batch_execute("COMMIT").await?;
    Ok(())
}

#[tokio::test]
async fn a_consumed_job_does_not_block_or_erase_a_new_generation() -> anyhow::Result<()> {
    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let painter = queue_client(&config.database_url).await?;
    let writer = queue_client(&config.database_url).await?;
    let expander = queue_client(&config.database_url).await?;
    let others = queue_client(&config.database_url).await?;
    let id = format!("generation-{}", uuid::Uuid::new_v4().simple());
    writer.execute("INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false)", &[&id]).await?;
    writer.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)", &[&id]).await?;
    writer
        .execute(
            "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$1,$1,'body')",
            &[&id],
        )
        .await?;
    isolate_queue(&others, &id).await?;
    painter
        .batch_execute("BEGIN; SELECT refresh_next_display()")
        .await?;
    writer
        .execute("UPDATE users SET display_name='After' WHERE id=$1", &[&id])
        .await?;
    expander
        .batch_execute("SET statement_timeout='1s'; SELECT expand_next_display_scope()")
        .await?;
    painter.batch_execute("COMMIT").await?;
    let queued: bool = writer
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM display_refresh_queue WHERE kind='message' AND target=$1)",
            &[&id],
        )
        .await?
        .get(0);
    assert!(queued, "new generation survives the old job's commit");
    drain_queue(&expander).await?;
    let name: String = writer
        .query_one(
            "SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1",
            &[&id],
        )
        .await?
        .get(0);
    assert_eq!(name, "After");
    writer
        .execute("DELETE FROM messages WHERE id=$1", &[&id])
        .await?;
    others.batch_execute("COMMIT").await?;
    Ok(())
}

#[tokio::test]
async fn timed_out_rows_and_scopes_do_not_starve_other_jobs_and_retry_with_more_time()
-> anyhow::Result<()> {
    use futures_util::FutureExt;

    let _guard = DISPLAY_TESTS.lock().await;
    let config = Config::from_env();
    let client = queue_client(&config.database_url).await?;
    let others = queue_client(&config.database_url).await?;
    let prefix = format!("retry-{}", uuid::Uuid::new_v4().simple());
    let slow = format!("{prefix}-a");
    let healthy = format!("{prefix}-z");
    let notice = format!("{prefix}-n");
    let scope = format!("{prefix}-scope");
    client
        .execute(
            "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,'Before')",
            &[&prefix],
        )
        .await?;
    client.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)", &[&prefix]).await?;
    for id in [&slow, &healthy] {
        client
            .execute(
                "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$2,'body')",
                &[id, &prefix],
            )
            .await?;
    }
    client.execute("INSERT INTO notifications(id,user_id,actor_id,channel_id,message_id,notification_type) VALUES($1,$2,$2,$2,$3,'mention')", &[&notice,&prefix,&healthy]).await?;
    client
        .execute(
            "UPDATE users SET display_name='After' WHERE id=$1",
            &[&prefix],
        )
        .await?;
    client
        .execute(
            "DELETE FROM display_refresh_queue WHERE starts_with(target,$1)",
            &[&prefix],
        )
        .await?;
    client.execute("INSERT INTO display_refresh_queue(kind,target) VALUES('message',$1),('message',$2),('notification',$3),('emoji',$4)", &[&slow,&healthy,&notice,&scope]).await?;
    isolate_queue(&others, &prefix).await?;
    let function = format!("test_delay_{}", uuid::Uuid::new_v4().simple());
    // Inject deterministic DB work instead of relying on CPU speed or a fixed
    // reaction count. Each delay exceeds its first worker budget (200ms/5s).
    client.batch_execute(&format!(
        "CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.target=TG_ARGV[0] THEN PERFORM pg_sleep(TG_ARGV[1]::double precision); END IF; RETURN OLD; END $$;
         CREATE TRIGGER {function}_row BEFORE DELETE ON display_refresh_queue FOR EACH ROW EXECUTE FUNCTION {function}('{slow}','0.35');
         CREATE TRIGGER {function}_scope BEFORE DELETE ON display_refresh_queue FOR EACH ROW EXECUTE FUNCTION {function}('{scope}','5.2');"
    )).await?;
    let state =
        backend::state::AppState::new(backend::db::init_db(&config.database_url).await?, config);
    let result = std::panic::AssertUnwindSafe(async {
        assert!(backend::display_refresh::refresh_one(&state).await?);
        for target in [&slow, &scope] {
            let row = client.query_one("SELECT attempts,retry_at>clock_timestamp() FROM display_refresh_queue WHERE target=$1", &[target]).await?;
            assert_eq!(row.get::<_,i32>(0), 1);
            assert!(row.get::<_,bool>(1), "failed work must be deferred and retained");
        }
        assert_eq!(client.query_one("SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1", &[&slow]).await?.get::<_,String>(0), "Before", "timed-out projection must roll back");
        client.execute("UPDATE display_refresh_queue SET retry_at=clock_timestamp()+interval '1 hour' WHERE target IN ($1,$2)", &[&slow,&scope]).await?;
        // A message repaint may enqueue a new notification generation as well.
        for _ in 0..8 {
            if !backend::display_refresh::refresh_one(&state).await? { break; }
        }
        let pending:i64=client.query_one("SELECT count(*) FROM display_refresh_queue WHERE target IN ($1,$2)", &[&healthy,&notice]).await?.get(0);
        assert_eq!(pending,0,"healthy message and notification must advance past both timeouts");
        let drift:i64=client.query_one("SELECT count(*) FROM messages m JOIN message_display v USING(id) WHERE m.id=$1 AND m.display_data IS DISTINCT FROM v.display_data", &[&healthy]).await?.get(0);
        assert_eq!(drift,0);
        let drift:i64=client.query_one("SELECT count(*) FROM notification_events n JOIN notification_display v USING(id) WHERE n.id=$1 AND n.display_data IS DISTINCT FROM v.display_data", &[&notice]).await?.get(0);
        assert_eq!(drift,0);
        // Make the row runnable without sleeping through backoff. Its second
        // budget is 400ms, enough for the 350ms repaint. Keep the scope deferred.
        client.execute("UPDATE display_refresh_queue SET retry_at=CASE WHEN target=$1 THEN '-infinity'::timestamptz ELSE clock_timestamp()+interval '1 hour' END WHERE target IN ($1,$2)", &[&slow,&scope]).await?;
        assert!(backend::display_refresh::refresh_one(&state).await?);
        assert!(!client.query_one("SELECT EXISTS(SELECT 1 FROM display_refresh_queue WHERE target=$1)", &[&slow]).await?.get::<_,bool>(0));
        assert_eq!(client.query_one("SELECT display_data->'author'->>'display_name' FROM messages WHERE id=$1", &[&slow]).await?.get::<_,String>(0),"After");
        client.execute("UPDATE display_refresh_queue SET retry_at='-infinity' WHERE target=$1", &[&scope]).await?;
        assert!(backend::display_refresh::refresh_one(&state).await?);
        assert!(!client.query_one("SELECT EXISTS(SELECT 1 FROM display_refresh_queue WHERE target=$1)", &[&scope]).await?.get::<_,bool>(0));
        anyhow::Ok(())
    }).catch_unwind().await;
    others.batch_execute("COMMIT").await?;
    client.batch_execute(&format!("DROP TRIGGER {function}_row ON display_refresh_queue; DROP TRIGGER {function}_scope ON display_refresh_queue; DROP FUNCTION {function}();")).await?;
    client
        .execute("DELETE FROM notifications WHERE id=$1", &[&notice])
        .await?;
    client
        .execute("DELETE FROM messages WHERE channel_id=$1", &[&prefix])
        .await?;
    result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
