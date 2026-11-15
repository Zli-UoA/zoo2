//! Staged migration against the real schema, isolated from other integration tests.
use backend::config::Config;

#[tokio::test]
async fn staged_backfill_preserves_leaves_links_and_archive_state() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let (mut client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(56220261128)")
        .await?;
    let schema = format!("thread_rollout_{}", uuid::Uuid::new_v4().simple());
    tx.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};"
    ))
    .await?;
    tx.batch_execute(include_str!("../../infra/db/schema.sql"))
        .await?;
    tx.batch_execute("CREATE TABLE app_events(name text);
        CREATE OR REPLACE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$ BEGIN INSERT INTO app_events VALUES(event_name); END $$;
        INSERT INTO users(id,user_name,display_name) VALUES ('a','a','A'),('b','b','B'),('c','c','C');
        INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES('channel','channel','a',false);
        INSERT INTO home_posts(id,user_id,channel_id,content,created_at) VALUES ('root','a','channel','root','2020-01-01');
        INSERT INTO messages(id,user_id,channel_id,content,created_at,parent_id) VALUES ('child','b','channel','child','2020-01-02','root');
        INSERT INTO messages(id,user_id,channel_id,content,created_at,parent_id) VALUES ('nested','c','channel','nested','2020-01-03','child');
        UPDATE messages SET thread_archive_at='2020-02-01' WHERE id='root';
        UPDATE thread_participants SET left_at='2020-02-02' WHERE thread_id='root' AND user_id='b';
        INSERT INTO home_posts(id,user_id,content) VALUES('home','a','Home');
        INSERT INTO home_posts(id,user_id,content,parent_id) VALUES('home-child','b','Reply','home');
        INSERT INTO home_posts(id,user_id,content,parent_id) VALUES('home-nested','c','Nested','home-child');").await?;
    assert_eq!(
        tx.query_one("SELECT parent_id FROM messages WHERE id='nested'", &[])
            .await?
            .get::<_, String>(0),
        "child"
    );
    // The additive deployment has not rewritten history; after old servers stop,
    // new writes normalize while historical roots are migrated in separate batches.
    tx.batch_execute("UPDATE thread_rollout SET phase='normalizing';
        INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES('during','a','channel','During rollout','nested');").await?;
    assert_eq!(
        tx.query_one("SELECT parent_id FROM messages WHERE id='during'", &[])
            .await?
            .get::<_, String>(0),
        "root"
    );
    let events: i64 = tx
        .query_one("SELECT count(*) FROM app_events", &[])
        .await?
        .get(0);
    assert_eq!(
        tx.query_one("SELECT migrate_thread_batch(1)", &[])
            .await?
            .get::<_, i32>(0),
        1
    );
    assert_eq!(
        tx.query_one("SELECT migrate_thread_batch(1)", &[])
            .await?
            .get::<_, i32>(0),
        0
    );
    assert_eq!(
        tx.query_one("SELECT count(*) FROM app_events", &[])
            .await?
            .get::<_, i64>(0),
        events
    );
    let row = tx
        .query_one(
            "SELECT parent_id,reply_target_id,content FROM messages WHERE id='nested'",
            &[],
        )
        .await?;
    assert_eq!(row.get::<_, String>(0), "root");
    assert_eq!(row.get::<_, String>(1), "child");
    assert_eq!(row.get::<_, String>(2), "nested");
    let original_home: String = tx
        .query_one(
            "SELECT reply_target_id FROM home_posts WHERE id='nested'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(original_home, "child");
    assert!(tx.query_one("SELECT left_at IS NOT NULL FROM thread_participants WHERE thread_id='root' AND user_id='b'", &[]).await?.get::<_, bool>(0));
    assert_eq!(
        tx.query_one(
            "SELECT count(*) FROM thread_participants WHERE thread_id='root'",
            &[]
        )
        .await?
        .get::<_, i64>(0),
        3
    );
    assert!(tx.query_one("SELECT thread_archive_at IS NULL AND display_data->>'reply_count'='3' FROM messages WHERE id='root'", &[]).await?.get::<_, bool>(0));
    assert_eq!(
        tx.query_one("SELECT migrate_home_thread_batch(1)", &[])
            .await?
            .get::<_, i32>(0),
        1
    );
    let home = tx
        .query_one(
            "SELECT parent_id,reply_target_id FROM home_posts WHERE id='home-nested'",
            &[],
        )
        .await?;
    assert_eq!(home.get::<_, String>(0), "home");
    assert_eq!(home.get::<_, String>(1), "home-child");
    assert_eq!(
        tx.query_one("SELECT migrate_home_thread_batch(1)", &[])
            .await?
            .get::<_, i32>(0),
        0
    );
    // Rerunning a completed root never revives a leave. A historical import also
    // keeps both the leave and archive, whereas the user's new post rejoins.
    tx.batch_execute("UPDATE messages SET thread_archive_at='2020-03-01' WHERE id='root';
        DELETE FROM thread_migration_completed;
        SELECT migrate_thread_batch(1);
        UPDATE thread_rollout SET phase='ready';
        INSERT INTO messages(id,user_id,channel_id,content,created_at,parent_id) VALUES('history','b','channel','History','2019-01-01','nested');").await?;
    assert!(
        tx.query_one(
            "SELECT thread_archive_at IS NOT NULL FROM messages WHERE id='root'",
            &[]
        )
        .await?
        .get::<_, bool>(0)
    );
    assert!(tx.query_one("SELECT left_at IS NOT NULL FROM thread_participants WHERE thread_id='root' AND user_id='b'", &[]).await?.get::<_, bool>(0));
    tx.batch_execute("INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES('live','b','channel','New','nested');").await?;
    assert!(
        tx.query_one(
            "SELECT thread_archive_at IS NULL FROM messages WHERE id='root'",
            &[]
        )
        .await?
        .get::<_, bool>(0)
    );
    assert!(tx.query_one("SELECT left_at IS NULL FROM thread_participants WHERE thread_id='root' AND user_id='b'", &[]).await?.get::<_, bool>(0));
    // A deleted ancestor can have replies but no activity marker. It still needs
    // flattening, even though the deleted root stays inaccessible to readers.
    tx.batch_execute("UPDATE thread_rollout SET phase='collecting';
        INSERT INTO messages(id,user_id,channel_id,content,deleted_at) VALUES('deleted-root','a','channel','Deleted',now());
        INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES('deleted-child','b','channel','Child','deleted-root');
        INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES('deleted-nested','c','channel','Nested','deleted-child');
        UPDATE thread_rollout SET phase='normalizing';
        SELECT migrate_thread_batch(1);").await?;
    assert_eq!(
        tx.query_one(
            "SELECT parent_id FROM messages WHERE id='deleted-nested'",
            &[]
        )
        .await?
        .get::<_, String>(0),
        "deleted-root"
    );
    tx.rollback().await?;
    drop(client);
    driver.await??;
    Ok(())
}

#[tokio::test]
async fn home_first_preserves_original_parent_and_encrypted_parents_never_change()
-> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let (mut client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let tx = client.transaction().await?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(56220261128)")
        .await?;
    let schema = format!("thread_guards_{}", uuid::Uuid::new_v4().simple());
    tx.batch_execute(&format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};"
    ))
    .await?;
    tx.batch_execute(include_str!("../../infra/db/schema.sql"))
        .await?;
    tx.batch_execute("INSERT INTO users(id,user_name,display_name) VALUES('a','a','A');
        INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES('plain','plain','a',false),('encrypted','encrypted','a',true);
        UPDATE channels SET encrypted=true WHERE id='encrypted';
        INSERT INTO home_posts(id,user_id,channel_id,content) VALUES('root','a','plain','Root');
        INSERT INTO home_posts(id,user_id,channel_id,content,parent_id) VALUES('child','a','plain','Child','root');
        INSERT INTO home_posts(id,user_id,channel_id,content,parent_id) VALUES('nested','a','plain','Nested','child');
        INSERT INTO messages(id,user_id,channel_id,content,ciphertext,epoch) VALUES('e-root','a','encrypted','','{}',1);
        INSERT INTO messages(id,user_id,channel_id,content,ciphertext,epoch,parent_id) VALUES('e-child','a','encrypted','','{}',1,'e-root');
        INSERT INTO messages(id,user_id,channel_id,content,ciphertext,epoch,parent_id) VALUES('e-nested','a','encrypted','','{}',1,'e-child');
        UPDATE thread_rollout SET phase='normalizing';
        SELECT migrate_home_thread_batch(100);").await?;
    let original = tx
        .query_one(
            "SELECT parent_id,reply_target_id FROM messages WHERE id='nested'",
            &[],
        )
        .await?;
    assert_eq!(original.get::<_, String>(0), "root");
    assert_eq!(original.get::<_, String>(1), "child");
    for sql in [
        "SELECT migrate_thread_batch(100)",
        "INSERT INTO messages(id,user_id,channel_id,content,ciphertext,epoch,parent_id) VALUES('e-new','a','encrypted','','{}',1,'e-child')",
        "UPDATE messages SET parent_id='e-root' WHERE id='e-nested'",
    ] {
        tx.batch_execute("SAVEPOINT guard").await?;
        let error = tx.batch_execute(sql).await.unwrap_err();
        assert!(error.as_db_error().unwrap().message().contains("Encrypted"));
        tx.batch_execute("ROLLBACK TO SAVEPOINT guard").await?;
    }
    assert_eq!(
        tx.query_one("SELECT parent_id FROM messages WHERE id='e-nested'", &[])
            .await?
            .get::<_, String>(0),
        "e-child"
    );
    // Ordinary encrypted root replies remain supported.
    tx.batch_execute("INSERT INTO messages(id,user_id,channel_id,content,ciphertext,epoch,parent_id) VALUES('e-direct','a','encrypted','','{}',1,'e-root')").await?;
    tx.rollback().await?;
    drop(client);
    driver.await??;
    Ok(())
}
