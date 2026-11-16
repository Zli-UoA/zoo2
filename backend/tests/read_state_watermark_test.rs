use backend::{config::Config, routes::read_states::states_sql};
use serde_json::Value;
use tokio_postgres::{NoTls, Transaction};

async fn unread(tx: &Transaction<'_>, viewer: &str, channel: &str) -> anyhow::Result<bool> {
    let row = tx
        .query_one(&states_sql(true), &[&viewer, &channel])
        .await?;
    let value: Value = serde_json::from_str(&row.get::<_, String>(0))?;
    Ok(value["has_unread"].as_bool().unwrap())
}

#[tokio::test]
async fn watermarks_preserve_unread_through_root_changes_and_ties() -> anyhow::Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let viewer = uuid::Uuid::new_v4().to_string();
    let author = format!("{viewer}-author");
    let renamed = format!("{author}-renamed");
    let channel = format!("{viewer}-channel");
    let other = format!("{viewer}-other");
    let own = format!("{viewer}-own");
    let old = format!("{viewer}-old");
    let fresh = format!("{viewer}-fresh");
    tx.execute("INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",&[&viewer,&author]).await?;
    tx.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$3,false),($2::text,$2::text,$3,false)",&[&channel,&other,&viewer]).await?;
    tx.execute("INSERT INTO channel_read_states(user_id,channel_id,last_read_at) VALUES($1,$2,'2020-01-01'),($1,$3,'2020-01-01')", &[&viewer,&channel,&other]).await?;
    tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$3,$5,'own','2030-01-01'),($2,$4,$5,'old','2000-01-01')", &[&own,&old,&viewer,&author,&channel]).await?;
    assert!(
        !unread(&tx, &viewer, &channel).await?,
        "own newer posts do not hide the older-author boundary"
    );
    tx.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'fresh','2040-01-01')", &[&fresh,&author,&channel]).await?;
    assert!(unread(&tx, &viewer, &channel).await?);
    tx.execute(
        "UPDATE messages SET created_at='2010-01-01' WHERE id=$1",
        &[&fresh],
    )
    .await?;
    assert!(!unread(&tx, &viewer, &channel).await?);
    tx.execute(
        "UPDATE messages SET created_at='2040-01-01',deleted_at=now() WHERE id=$1",
        &[&fresh],
    )
    .await?;
    assert!(!unread(&tx, &viewer, &channel).await?);
    tx.execute("UPDATE messages SET deleted_at=NULL WHERE id=$1", &[&fresh])
        .await?;
    assert!(unread(&tx, &viewer, &channel).await?);
    tx.execute(
        "UPDATE messages SET parent_id=$2 WHERE id=$1",
        &[&fresh, &own],
    )
    .await?;
    assert!(
        !unread(&tx, &viewer, &channel).await?,
        "replies are not channel unread roots"
    );
    tx.execute(
        "UPDATE messages SET parent_id=NULL,channel_id=$2 WHERE id=$1",
        &[&fresh, &other],
    )
    .await?;
    assert!(!unread(&tx, &viewer, &channel).await?);
    assert!(unread(&tx, &viewer, &other).await?);
    tx.execute(
        "UPDATE messages SET user_id=$2 WHERE id=$1",
        &[&fresh, &viewer],
    )
    .await?;
    assert!(!unread(&tx, &viewer, &other).await?);
    tx.execute(
        "UPDATE messages SET user_id=$2 WHERE id=$1",
        &[&fresh, &author],
    )
    .await?;
    tx.execute("UPDATE users SET id=$2 WHERE id=$1", &[&author, &renamed])
        .await?;
    assert!(
        unread(&tx, &viewer, &other).await?,
        "FK cascades keep the author watermark"
    );
    tx.execute("DELETE FROM messages WHERE id=$1", &[&fresh])
        .await?;
    assert!(!unread(&tx, &viewer, &other).await?);
    tx.execute(
        "UPDATE messages SET created_at='2030-01-01' WHERE id=$1",
        &[&old],
    )
    .await?;
    assert!(
        unread(&tx, &viewer, &channel).await?,
        "two tied authors must both be considered"
    );
    tx.execute(
        "UPDATE channel_read_states SET last_read_at='2030-01-01' WHERE user_id=$1",
        &[&viewer],
    )
    .await?;
    assert!(
        !unread(&tx, &viewer, &channel).await?,
        "strict timestamp read boundary is unchanged"
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn a_source_writer_waits_for_the_watermark_recount_and_then_updates_it() -> anyhow::Result<()>
{
    let url = Config::from_env().database_url;
    let (mut owner, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let (writer, connection) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(connection);
    let id = uuid::Uuid::new_v4().to_string();
    let fresh = format!("{id}-new");
    owner
        .execute(
            "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
            &[&id],
        )
        .await?;
    owner.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)",&[&id]).await?;
    owner.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$1,$1,'old','2025-01-01')", &[&id]).await?;
    let pid: i32 = writer
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let tx = owner.transaction().await?;
    tx.query_one("SELECT latest_at FROM channel_message_authors WHERE channel_id=$1 AND user_id=$1 FOR UPDATE", &[&id]).await?;
    let target = id.clone();
    let post = fresh.clone();
    let task = tokio::spawn(async move {
        writer.execute("INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$2,'new','2035-01-01')", &[&post,&target]).await
    });
    let mut blocked = false;
    for _ in 0..100 {
        if tx
            .query_one(
                "SELECT wait_event_type='Lock' FROM pg_stat_activity WHERE pid=$1",
                &[&pid],
            )
            .await?
            .get::<_, Option<bool>>(0)
            == Some(true)
        {
            blocked = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        blocked,
        "source writer must wait before committing its watermark"
    );
    tx.execute("UPDATE channel_message_authors SET latest_at=(SELECT created_at FROM messages WHERE channel_id=$1 AND user_id=$1 AND parent_id IS NULL AND deleted_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1) WHERE channel_id=$1 AND user_id=$1", &[&id]).await?;
    tx.commit().await?;
    task.await??;
    assert!(owner.query_one("SELECT latest_at='2035-01-01'::timestamp FROM channel_message_authors WHERE channel_id=$1 AND user_id=$1", &[&id]).await?.get::<_,bool>(0));
    owner
        .execute("DELETE FROM messages WHERE channel_id=$1", &[&id])
        .await?;
    owner
        .execute("DELETE FROM channels WHERE id=$1", &[&id])
        .await?;
    owner
        .execute("DELETE FROM users WHERE id=$1", &[&id])
        .await?;
    Ok(())
}
