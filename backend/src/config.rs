use std::{env, fmt};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database_url", &"[REDACTED]")
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Self {
        // .env または親ディレクトリの .env を読み込む
        dotenvy::dotenv().ok();
        dotenvy::from_filename("../.env").ok();

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "root".to_string());
            let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string());
            let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
            let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
            let db = env::var("POSTGRES_DB").unwrap_or_else(|_| "zoo".to_string());
            format!("postgresql://{user}:{password}@{host}:{port}/{db}?sslmode=disable")
        });

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8000);

        Self {
            database_url,
            host,
            port,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_debug_redacts_database_url() {
        let config = Config {
            database_url: "postgresql://user:super_secret_password@localhost:5432/zoo".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
        };
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("super_secret_password"));
        assert!(!debug_str.contains("user"));
        assert!(debug_str.contains("[REDACTED]"));
        assert!(debug_str.contains("8080"));
    }
}
