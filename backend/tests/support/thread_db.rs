//! Each HTTP test gets its own rollout singleton and schema.
use backend::config::Config;

pub struct ThreadDb {
    pub config: Config,
    admin: tokio_postgres::Client,
    /// The connection string without the schema's `search_path`, for dropping the schema.
    admin_url: String,
    schema: String,
    cleaned: bool,
}

impl ThreadDb {
    pub async fn new() -> anyhow::Result<Self> {
        let mut config = Config::from_env();
        config.dev_auth_enabled = true;
        let (mut admin, connection) =
            tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(connection);
        let admin_url = config.database_url.clone();
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
            admin_url,
            schema,
            cleaned: false,
        })
    }

    pub async fn cleanup(mut self) -> anyhow::Result<()> {
        self.admin
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await?;
        self.cleaned = true;
        Ok(())
    }
}

/// A test that fails (an assertion, an error, a panic) never reaches `cleanup`: the schema is
/// dropped here, on a thread and a connection of its own, so it does not outlive the test.
impl Drop for ThreadDb {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        let (url, schema) = (self.admin_url.clone(), self.schema.clone());
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                if let Ok((client, connection)) =
                    tokio_postgres::connect(&url, tokio_postgres::NoTls).await
                {
                    tokio::spawn(connection);
                    let _ = client
                        .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                        .await;
                }
            });
        })
        .join();
    }
}
