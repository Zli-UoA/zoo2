use crate::{auth::oidc::JwksVerifier, config::Config};

#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    pub config: Config,
    pub http_client: reqwest::Client,
    pub jwks_verifier: JwksVerifier,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .tcp_keepalive(Some(std::time::Duration::from_secs(30)))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let jwks_verifier = JwksVerifier::new(http_client.clone());

        Self {
            db,
            config,
            http_client,
            jwks_verifier,
        }
    }

    pub fn with_client(db: toasty::Db, config: Config, http_client: reqwest::Client) -> Self {
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        Self {
            db,
            config,
            http_client,
            jwks_verifier,
        }
    }

    pub fn with_verifier(
        db: toasty::Db,
        config: Config,
        http_client: reqwest::Client,
        jwks_verifier: JwksVerifier,
    ) -> Self {
        Self {
            db,
            config,
            http_client,
            jwks_verifier,
        }
    }
}
