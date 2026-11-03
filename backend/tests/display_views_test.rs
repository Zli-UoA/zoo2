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
