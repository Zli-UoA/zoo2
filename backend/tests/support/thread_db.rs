//! Each HTTP test gets its own rollout singleton and schema.
use backend::config::Config;

pub struct ThreadDb {
    pub config: Config,
    admin: tokio_postgres::Client,
    schema: String,
}

impl ThreadDb {
    pub async fn new() -> anyhow::Result<Self> {
        let mut config = Config::from_env();
        config.dev_auth_enabled = true;
        let (mut admin, connection) =
            tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(connection);
        let schema = format!("thread_http_{}", uuid::Uuid::new_v4().simple());
        let tx = admin.transaction().await?;
        // schema.sql also grants shared role privileges; serialize that DDL.
        tx.batch_execute("SELECT pg_advisory_xact_lock(56220261128)")
            .await?;
        tx.batch_execute(&format!(
            "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};"
        ))
        .await?;
        tx.batch_execute(include_str!("../../../infra/db/schema.sql"))
            .await?;
        tx.commit().await?;
        let mut url = reqwest::Url::parse(&config.database_url)?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        config.database_url = url.to_string();
        Ok(Self {
            config,
            admin,
            schema,
        })
    }

    pub async fn cleanup(self) -> anyhow::Result<()> {
        self.admin
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await?;
        Ok(())
    }
}
