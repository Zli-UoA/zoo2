use backend::config::Config;
use tokio_postgres::NoTls;

#[tokio::test]
async fn simultaneous_reactions_do_not_deadlock_on_foreign_key_locks() -> anyhow::Result<()> {
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
async fn metadata_refresh_and_reply_edits_take_the_gate_before_row_locks() -> anyhow::Result<()> {
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (observer, oc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    tokio::spawn(oc);
    let prefix = format!("gate-{}", uuid::Uuid::new_v4().simple());
    let parent = format!("{prefix}-a-parent");
    let child = format!("{prefix}-z-child");
    a.execute(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Before',false)",
        &[&prefix],
    )
    .await?;
    a.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",
        &[&prefix],
    )
    .await?;
    a.execute(
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1::text,$2::text,$2::text,':gate:')",
        &[&parent, &prefix],
    )
    .await?;
    a.execute("INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES($1::text,$2::text,$2::text,':gate:',$3::text)", &[&child,&prefix,&parent]).await?;
    a.execute(
        "INSERT INTO workspace_emojis(name,url,created_by) VALUES($1::text,'/gate.png',$1::text)",
        &[&prefix],
    )
    .await?;
    // Give both rows an actual emoji dependency, regardless of UUID in the name.
    a.execute(
        "UPDATE messages SET content=':'||$1::text||':' WHERE channel_id=$1::text",
        &[&prefix],
    )
    .await?;
    let pid: i32 = b.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    for metadata_first in [false, true] {
        for profile in [false, true] {
            let metadata = if profile {
                format!("UPDATE users SET display_name=display_name||'x' WHERE id='{prefix}'")
            } else {
                format!(
                    "UPDATE workspace_emojis SET width=COALESCE(width,0)+1 WHERE name='{prefix}'"
                )
            };
            let edit = format!("UPDATE messages SET content=content||' edit' WHERE id='{child}'");
            let (first, second) = if metadata_first {
                (&metadata, &edit)
            } else {
                (&edit, &metadata)
            };
            a.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
                .await?;
            b.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
                .await?;
            a.batch_execute(first).await?;
            let write = b.batch_execute(second);
            let release = async {
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    loop {
                        let waiting: bool = observer.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)", &[&pid]).await?.get(0);
                        if waiting { break Ok::<_,tokio_postgres::Error>(()); }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }).await??;
                a.batch_execute("COMMIT").await?;
                Ok::<_, anyhow::Error>(())
            };
            let (written, released) = tokio::join!(write, release);
            released?;
            written?;
            b.batch_execute("COMMIT").await?;
            let drift: i64 = observer.query_one("SELECT count(*) FROM messages m JOIN message_display v USING(id) WHERE m.channel_id=$1 AND m.display_data IS DISTINCT FROM v.display_data", &[&prefix]).await?.get(0);
            assert_eq!(
                drift, 0,
                "metadata_first={metadata_first}, profile={profile}"
            );
        }
    }
    a.execute("DELETE FROM messages WHERE channel_id=$1", &[&prefix])
        .await?;
    a.execute("DELETE FROM workspace_emojis WHERE name=$1", &[&prefix])
        .await?;
    a.execute(
        "UPDATE channels SET deleted_at=now() WHERE id=$1",
        &[&prefix],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn canceled_phase_entry_does_not_leak_session_locks() -> anyhow::Result<()> {
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    a.batch_execute("BEGIN; SELECT enter_display_group('metadata')")
        .await?;
    b.batch_execute("BEGIN; SET LOCAL statement_timeout='100ms'")
        .await?;
    let error = b
        .batch_execute("SELECT enter_display_group('row')")
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
    );
    b.batch_execute("ROLLBACK").await?;
    let leaked: i64 = b.query_one("SELECT count(*) FROM pg_locks WHERE pid=pg_backend_pid() AND locktype='advisory' AND classid=2063231026 AND granted", &[]).await?.get(0);
    assert_eq!(leaked, 0);
    a.batch_execute("COMMIT").await?;
    b.batch_execute(
        "BEGIN; SET LOCAL statement_timeout='1s'; SELECT enter_display_group('row'); COMMIT",
    )
    .await?;
    Ok(())
}
