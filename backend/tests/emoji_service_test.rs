use anyhow::Context;
use backend::{config::Config, emoji_service::project_catalog, state::AppState};
use serde_json::json;
use tokio_postgres::NoTls;

#[tokio::test]
#[ignore = "requires dedicated EMOJI_TEST_DATABASE_URL; run with --include-ignored"]
async fn shared_projection_updates_joined_display_and_preserves_local_registry()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.database_url = std::env::var("EMOJI_TEST_DATABASE_URL")
        .context("Set EMOJI_TEST_DATABASE_URL to a disposable emoji_projection_test database")?;
    anyhow::ensure!(
        reqwest::Url::parse(&config.database_url)?.path() == "/emoji_projection_test",
        "A dedicated emoji_projection_test database is required"
    );
    let (client, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let connection = tokio::spawn(connection);
    let state = AppState::new(backend::db::init_db(&config.database_url).await?, config);
    client
        .execute("DELETE FROM shared_emoji_source", &[])
        .await?;
    let id = format!("shared-{}", uuid::Uuid::new_v4().simple());
    let name = format!("shared_{}", uuid::Uuid::new_v4().simple());
    client
        .execute(
            "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,'Shared emoji test')",
            &[&id],
        )
        .await?;
    client.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$1::text,false)", &[&id]).await?;
    client
        .execute(
            "INSERT INTO workspace_emojis(name,url,created_by) VALUES($1,'/legacy.png',$2),($1||'_legacy','/legacy-only.png',$2)",
            &[&name, &id],
        )
        .await?;
    client
        .execute(
            "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1::text,$1::text,$1::text,$2::text)",
            &[&id, &format!(":{name}: :ねこ: :{name}_legacy:")],
        )
        .await?;
    client.execute("INSERT INTO notification_events(id,user_id,actor_id,channel_id,message_id,notification_type) VALUES($1::text,$1::text,$1::text,$1::text,$1::text,'mention')", &[&id]).await?;
    let source = reqwest::Url::parse("https://emoji.example.test")?;
    let image = format!("https://emoji.example.test/images/{}", "a".repeat(64));
    let rows = vec![
        json!({"name":name,"url":image,"category":"animals","aliases":["ねこ"],"width":48,"height":24,"image_id":"a".repeat(64)}),
    ];
    project_catalog(&state, Some(&source), &rows).await?;
    for _ in 0..1000 {
        if !backend::display_refresh::refresh_one(&state).await? {
            break;
        }
    }
    let value = client.query_one("SELECT display_data->'emojis'->$2->>'url',(display_data->'emojis'->$2->>'width')::integer,display_data->'emojis'->'ねこ'->>'url' FROM messages WHERE id=$1",&[&id,&name]).await?;
    assert_eq!(value.get::<_, String>(0), image);
    assert_eq!(value.get::<_, i32>(1), 48);
    assert_eq!(value.get::<_, String>(2), image);
    let notice: String = client.query_one("SELECT display_data->'message'->'emojis'->$2->>'url' FROM notification_events WHERE id=$1",&[&id,&name]).await?.get(0);
    assert_eq!(notice, image);
    let legacy_present: bool = client
        .query_one(
            "SELECT (display_data->'emojis') ? ($2||'_legacy') FROM messages WHERE id=$1",
            &[&id, &name],
        )
        .await?
        .get(0);
    assert!(
        !legacy_present,
        "initial source activation must invalidate local-only emoji displays"
    );
    let generation: i64 = client
        .query_one("SELECT count(*) FROM display_refresh_queue", &[])
        .await?
        .get(0);
    project_catalog(&state, Some(&source), &rows).await?;
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM display_refresh_queue", &[])
            .await?
            .get::<_, i64>(0),
        generation
    );
    let preserved: String = client
        .query_one("SELECT url FROM workspace_emojis WHERE name=$1", &[&name])
        .await?
        .get(0);
    assert_eq!(preserved, "/legacy.png");
    project_catalog(&state, None, &[]).await?;
    for _ in 0..1000 {
        if !backend::display_refresh::refresh_one(&state).await? {
            break;
        }
    }
    let local: String = client
        .query_one(
            "SELECT display_data->'emojis'->$2->>'url' FROM messages WHERE id=$1",
            &[&id, &name],
        )
        .await?
        .get(0);
    assert_eq!(local, "/legacy.png");
    client
        .execute("DELETE FROM notification_events WHERE id=$1", &[&id])
        .await?;
    client
        .execute("DELETE FROM messages WHERE id=$1", &[&id])
        .await?;
    client
        .execute(
            "DELETE FROM workspace_emojis WHERE name=$1 OR name=$1||'_legacy'",
            &[&name],
        )
        .await?;
    client
        .execute("DELETE FROM channels WHERE id=$1", &[&id])
        .await?;
    client
        .execute("DELETE FROM users WHERE id=$1", &[&id])
        .await?;
    connection.abort();
    Ok(())
}
