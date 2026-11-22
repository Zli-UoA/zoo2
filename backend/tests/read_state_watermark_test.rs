use backend::{config::Config, routes::read_states::states_sql};
use serde_json::Value;
use tokio_postgres::{NoTls, Transaction};

async fn state(tx: &Transaction<'_>, viewer: &str, channel: &str) -> anyhow::Result<Value> {
    let row = tx
        .query_one(&states_sql(true, true), &[&viewer, &channel])
        .await?;
    Ok(serde_json::from_str(&row.get::<_, String>(0))?)
}

async fn unread(tx: &Transaction<'_>, viewer: &str, channel: &str) -> anyhow::Result<bool> {
    Ok(state(tx, viewer, channel).await?["has_unread"]
        .as_bool()
        .unwrap())
}

#[tokio::test]
async fn unread_count_and_first_unread_are_bounded_by_the_read_boundary() -> anyhow::Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let viewer = uuid::Uuid::new_v4().to_string();
    let author = format!("{viewer}-author");
    let channel = format!("{viewer}-channel");
    let dm = format!("{viewer}-dm");
    let hidden = format!("{viewer}-hidden");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",
        &[&viewer, &author],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1::text,$1::text,$3,false,false),($2::text,$2::text,$3,false,true),($4::text,$4::text,$3,false,false)",
        &[&channel, &dm, &viewer, &hidden],
    )
    .await?;
    for id in [&channel, &dm] {
        for user in [&viewer, &author] {
            tx.execute(
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
                &[user, id],
            )
            .await?;
        }
    }
    // No read state yet: every other author's root message is unread.
    for (n, when) in [(1, "2025-01-01 00:00:01"), (2, "2025-01-01 00:00:02")] {
        tx.execute(
            "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'post',$4::text::timestamp)",
            &[&format!("{viewer}-m{n}"), &author, &channel, &when],
        )
        .await?;
    }
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'reply','2025-01-01 00:00:03')",
        &[&format!("{viewer}-reply"), &author, &channel],
    )
    .await?;
    tx.execute(
        "UPDATE messages SET parent_id=$1 WHERE id=$2",
        &[&format!("{viewer}-m1"), &format!("{viewer}-reply")],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'own','2025-01-01 00:00:04')",
        &[&format!("{viewer}-own"), &viewer, &channel],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at,deleted_at) VALUES($1,$2,$3,'deleted','2025-01-01 00:00:05',now())",
        &[&format!("{viewer}-deleted"), &author, &channel],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'dm','2025-01-01 00:00:06')",
        &[&format!("{viewer}-dm-message"), &author, &dm],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'hidden','2025-01-01 00:00:07')",
        &[&format!("{viewer}-hidden-message"), &author, &hidden],
    )
    .await?;
    let before = state(&tx, &viewer, &channel).await?;
    assert_eq!(before["unread_count"], 2);
    assert!(
        before["first_unread_at"]
            .as_str()
            .unwrap()
            .contains("00:00:01")
    );
    assert_eq!(state(&tx, &viewer, &dm).await?["unread_count"], 1);
    let listed = tx.query(&states_sql(false, true), &[&viewer]).await?;
    assert_eq!(listed.len(), 2, "non-member channels stay hidden");
    tx.execute(
        "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) VALUES($1,$2,'2025-01-01 00:00:01')",
        &[&viewer, &channel],
    )
    .await?;
    let after = state(&tx, &viewer, &channel).await?;
    assert_eq!(after["unread_count"], 1);
    assert!(
        after["first_unread_at"]
            .as_str()
            .unwrap()
            .contains("00:00:02")
    );
    tx.execute(
        "UPDATE channel_read_states SET last_read_at='2025-01-01 00:00:02' WHERE user_id=$1 AND channel_id=$2",
        &[&viewer, &channel],
    )
    .await?;
    let done = state(&tx, &viewer, &channel).await?;
    assert_eq!(done["has_unread"], false);
    assert_eq!(done["unread_count"], 0);
    assert!(done["first_unread_at"].is_null());
    tx.rollback().await?;
    Ok(())
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
async fn unread_details_skip_own_posts_and_tied_timestamps() -> anyhow::Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let viewer = uuid::Uuid::new_v4().to_string();
    let author = format!("{viewer}-author");
    let channel = format!("{viewer}-channel");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text),($2::text,$2::text,$2::text)",
        &[&viewer, &author],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&channel, &viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) VALUES($1,$2,'2025-01-01')",
        &[&viewer, &channel],
    )
    .await?;
    // Own posts and tied timestamps must not hide or misplace the other author's messages.
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT $1||'-own-'||n,$1,$2,'own','2025-01-02' FROM generate_series(1,1200)n",
        &[&viewer, &channel],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1||'-a',$2,$3,'first','2025-01-03'),($1||'-b',$2,$3,'tied','2025-01-03')",
        &[&viewer, &author, &channel],
    )
    .await?;
    let state = state(&tx, &viewer, &channel).await?;
    assert_eq!(state["unread_count"], 2);
    assert_eq!(state["unread_count_truncated"], false);
    assert!(
        state["first_unread_at"]
            .as_str()
            .unwrap()
            .contains("2025-01-03")
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn first_unread_is_not_limited_by_the_author_count() -> anyhow::Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let viewer = uuid::Uuid::new_v4().to_string();
    let channel = format!("{viewer}-channel");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&channel, &viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) VALUES($1,$2,'2025-01-01')",
        &[&viewer, &channel],
    )
    .await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) SELECT $1||'-author-'||n,$1||'-author-'||n,'Author' FROM generate_series(1,101)n",
        &[&viewer],
    )
    .await?;
    // The oldest unread author has the lowest latest_at, so the author-limit
    // ordering would drop it from the count query entirely.
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT $1||'-msg-'||n,$1||'-author-'||n,$2,'post',CASE WHEN n=1 THEN '2025-01-01 00:00:01' ELSE '2025-01-03 00:00:00' END::text::timestamp FROM generate_series(1,101)n",
        &[&viewer, &channel],
    )
    .await?;
    let state = state(&tx, &viewer, &channel).await?;
    assert_eq!(state["unread_count_truncated"], true);
    assert!(
        state["first_unread_at"]
            .as_str()
            .unwrap()
            .contains("2025-01-01")
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn exactly_one_hundred_unread_authors_is_not_truncated() -> anyhow::Result<()> {
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    let viewer = uuid::Uuid::new_v4().to_string();
    let channel = format!("{viewer}-channel");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&channel, &viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) SELECT $1||'-author-'||n,$1||'-author-'||n,'Author' FROM generate_series(1,100)n",
        &[&viewer],
    )
    .await?;
    tx.execute(
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) SELECT $1||'-msg-'||n,$1||'-author-'||n,$2,'post','2025-01-02' FROM generate_series(1,100)n",
        &[&viewer, &channel],
    )
    .await?;
    let state = state(&tx, &viewer, &channel).await?;
    assert_eq!(state["unread_count"], 100);
    assert_eq!(state["unread_count_truncated"], false);
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
