use backend::config::Config;
use tokio_postgres::NoTls;

#[tokio::test]
async fn notification_message_visibility_follows_legacy_and_canonical_writers() -> anyhow::Result<()>
{
    let config = Config::from_env();
    let (client, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let task = tokio::spawn(connection);
    let prefix = format!("visibility-{}-", uuid::Uuid::new_v4().simple());
    let sql = include_str!("notification_visibility.sql").replace("visibility-", &prefix);
    let result = client.batch_execute(&sql).await;
    drop(client);
    task.await??;
    result?;
    Ok(())
}

#[tokio::test]
async fn concurrent_notification_insert_and_message_visibility_change_stay_consistent()
-> anyhow::Result<()> {
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    for insert_first in [false, true] {
        for deleted in [false, true] {
            let id = format!("visibility-{}", uuid::Uuid::new_v4().simple());
            a.execute("INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Visibility',false)", &[&id]).await?;
            a.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)", &[&id]).await?;
            a.execute("INSERT INTO messages(id,user_id,channel_id,content,deleted_at) VALUES($1,$1,$1,'Visibility',CASE WHEN $2::boolean THEN NULL ELSE CURRENT_TIMESTAMP END)", &[&id,&deleted]).await?;
            let insert = format!(
                "INSERT INTO notifications(id,user_id,actor_id,channel_id,message_id,notification_type) VALUES('{id}','{id}','{id}','{id}','{id}','mention')"
            );
            let update = format!(
                "UPDATE messages SET deleted_at={} WHERE id='{id}'",
                if deleted { "CURRENT_TIMESTAMP" } else { "NULL" }
            );
            let (first, second) = if insert_first {
                (&insert, &update)
            } else {
                (&update, &insert)
            };
            a.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
                .await?;
            a.batch_execute(first).await?;
            let pid: i32 = b.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
            b.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
                .await?;
            // Poll the lock graph instead of depending on sleeps or scheduling.
            let blocked = async {
                tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    loop {
                        let waiting: bool = a
                            .query_one("SELECT cardinality(pg_blocking_pids($1)) > 0", &[&pid])
                            .await?
                            .get(0);
                        if waiting {
                            return anyhow::Ok(());
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await??;
                a.batch_execute("COMMIT").await?;
                anyhow::Ok(())
            };
            let write = async {
                b.batch_execute(second).await?;
                b.batch_execute("COMMIT").await?;
                anyhow::Ok(())
            };
            let (blocked, write) = tokio::join!(blocked, write);
            // Release locks even if the observation or writer failed.
            a.batch_execute("ROLLBACK").await?;
            b.batch_execute("ROLLBACK").await?;
            blocked?;
            write?;
            let actual: bool = a
                .query_one(
                    "SELECT message_deleted FROM notification_events WHERE id=$1",
                    &[&id],
                )
                .await?
                .get(0);
            assert_eq!(
                actual, deleted,
                "insert_first={insert_first}, deleted={deleted}"
            );
            a.execute("DELETE FROM messages WHERE id=$1", &[&id])
                .await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn read_receipt_bridge_does_not_lock_the_parent_message() -> anyhow::Result<()> {
    let config = Config::from_env();
    let (a, ac) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let (b, bc) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    tokio::spawn(ac);
    tokio::spawn(bc);
    let id = format!("visibility-{}", uuid::Uuid::new_v4().simple());
    a.execute(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1::text,$1::text,'Visibility',false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",
        &[&id],
    )
    .await?;
    a.execute(
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$1,$1,'Visibility')",
        &[&id],
    )
    .await?;
    a.execute("INSERT INTO notifications(id,user_id,actor_id,channel_id,message_id,notification_type) VALUES($1,$1,$1,$1,$1,'mention')", &[&id]).await?;
    a.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
        .await?;
    // A read receipt owns the inbox row while B soft-deletes its parent.
    a.query_one(
        "SELECT id FROM notification_events WHERE id=$1 FOR UPDATE",
        &[&id],
    )
    .await?;
    let pid: i32 = b.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    b.batch_execute("BEGIN; SET LOCAL statement_timeout='5s'")
        .await?;
    let receipt = async {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let blocked: bool = a
                    .query_one("SELECT cardinality(pg_blocking_pids($1)) > 0", &[&pid])
                    .await?
                    .get(0);
                if blocked {
                    return anyhow::Ok(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        // Its bridge must not request SHARE on the message that B owns.
        a.execute(
            "UPDATE notification_events SET is_read=true WHERE id=$1",
            &[&id],
        )
        .await?;
        a.batch_execute("COMMIT").await?;
        anyhow::Ok(())
    };
    let deletion = async {
        b.execute(
            "UPDATE messages SET deleted_at=CURRENT_TIMESTAMP WHERE id=$1",
            &[&id],
        )
        .await?;
        b.batch_execute("COMMIT").await?;
        anyhow::Ok(())
    };
    let (receipt, deletion) = tokio::join!(receipt, deletion);
    a.batch_execute("ROLLBACK").await?;
    b.batch_execute("ROLLBACK").await?;
    receipt?;
    deletion?;
    let row = a
        .query_one(
            "SELECT is_read,message_deleted FROM notification_events WHERE id=$1",
            &[&id],
        )
        .await?;
    assert!(row.get::<_, bool>(0));
    assert!(row.get::<_, bool>(1));
    a.execute("DELETE FROM messages WHERE id=$1", &[&id])
        .await?;
    Ok(())
}
