use backend::config::Config;
use tokio_postgres::NoTls;
use uuid::Uuid;

#[tokio::test]
async fn upgrades_existing_webhooks_without_replacing_identities_or_secrets() -> anyhow::Result<()>
{
    let config = Config::from_env();
    let (admin, connection) = tokio_postgres::connect(&config.database_url, NoTls).await?;
    let admin_task = tokio::spawn(connection);
    let database = format!("apps_migration_{}", Uuid::new_v4().simple());
    // A separate throwaway database isolates the pre-upgrade schema from other tests.
    admin
        .batch_execute(&format!("CREATE DATABASE {database}"))
        .await?;
    let mut settings: tokio_postgres::Config = config.database_url.parse()?;
    settings.dbname(&database);
    let (client, connection) = settings.connect(NoTls).await?;
    let task = tokio::spawn(connection);
    let mut migrations = std::fs::read_dir("../infra/db/migrations")?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    migrations.retain(|p| p.extension().is_some_and(|s| s == "sql"));
    migrations.sort();
    for file in &migrations {
        if file.file_name().unwrap().to_string_lossy().as_ref() < "20261027000000_create_apps.sql" {
            client
                .batch_execute(&std::fs::read_to_string(file)?)
                .await?;
        }
    }
    let human = Uuid::new_v4().to_string();
    let bot = Uuid::new_v4().to_string();
    let retained = Uuid::new_v4().to_string();
    let channel = Uuid::new_v4().to_string();
    let hook = Uuid::new_v4().to_string();
    let message = Uuid::new_v4().to_string();
    client.execute("INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1,'human','Human',false),($2,'webhook_existing','GitHub',true),($3,'webhook_deleted','Old integration',true)",&[&human,&bot,&retained]).await?;
    client.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,'existing',$2,true)",&[&channel,&human]).await?;
    client.execute("INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)",&[&channel,&bot]).await?;
    client.execute("INSERT INTO channel_webhooks(id,channel_id,name,secret,user_id) VALUES($1,$2,'Deploy setting','preserved-secret',$3)",&[&hook,&channel,&bot]).await?;
    client.execute("INSERT INTO messages(id,user_id,channel_id,content,card) VALUES($1,$2,$3,'Past notification','{\"title\":\"Past notification\"}')",&[&message,&bot,&channel]).await?;
    client
        .batch_execute(include_str!(
            "../../infra/db/migrations/20261027000000_create_apps.sql"
        ))
        .await?;
    client
        .batch_execute(include_str!(
            "../../infra/db/migrations/20261027010000_complete_app_events.sql"
        ))
        .await?;
    let row=client.query_one("SELECT w.id,w.secret,w.user_id,u.app_id,a.name,m.user_id FROM channel_webhooks w JOIN users u ON u.id=w.user_id JOIN apps a ON a.id=u.app_id CROSS JOIN messages m WHERE w.id=$1 AND m.id=$2",&[&hook,&message]).await?;
    assert_eq!(row.get::<_, String>(0), hook);
    assert_eq!(row.get::<_, String>(1), "preserved-secret");
    assert_eq!(row.get::<_, String>(2), bot);
    assert_eq!(row.get::<_, String>(3), bot);
    assert_eq!(row.get::<_, String>(4), "GitHub");
    assert_eq!(row.get::<_, String>(5), bot);
    assert_eq!(
        client
            .query_one("SELECT bot_user_id FROM apps WHERE id=$1", &[&retained])
            .await?
            .get::<_, String>(0),
        retained
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM users", &[])
            .await?
            .get::<_, i64>(0),
        3
    );
    drop(client);
    task.abort();
    admin
        .batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .await?;
    drop(admin);
    admin_task.abort();
    Ok(())
}
