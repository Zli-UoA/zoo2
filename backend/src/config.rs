use std::{env, fmt};

#[derive(Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: Option<String>,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("jwks_url", &self.jwks_url)
            .finish()
    }
}

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub electric_url: String,
    pub electric_secret: Option<String>,
    pub oidc: Option<OidcConfig>,
    pub dev_auth_enabled: bool,
    pub environment: String,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("electric_url", &self.electric_url)
            .field(
                "electric_secret",
                &self.electric_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("database_url", &"[REDACTED]")
            .field("oidc", &self.oidc)
            .field("dev_auth_enabled", &self.dev_auth_enabled)
            .field("environment", &self.environment)
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

        let electric_url = env::var("ELECTRIC_URL").unwrap_or_else(|_| {
            let host = env::var("ELECTRIC_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
            let port = env::var("ELECTRIC_PORT").unwrap_or_else(|_| "3000".to_string());
            format!("http://{host}:{port}")
        });

        let electric_secret = env::var("ELECTRIC_SECRET").ok().filter(|s| !s.is_empty());

        let oidc = match (
            env::var("OIDC_ISSUER"),
            env::var("OIDC_AUDIENCE").or_else(|_| env::var("OIDC_CLIENT_ID")),
        ) {
            (Ok(issuer), Ok(audience)) => {
                let jwks_url = env::var("OIDC_JWKS_URL").ok().filter(|s| !s.is_empty());

                Some(OidcConfig {
                    issuer,
                    audience,
                    jwks_url,
                })
            }
            _ => None,
        };

        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
        let dev_auth_env = env::var("DEV_AUTH_ENABLED").ok();
        #[cfg(test)]
        let is_test_build = true;
        #[cfg(not(test))]
        let is_test_build = false;

        let dev_auth_enabled = crate::auth::is_dev_auth_enabled_pure(
            dev_auth_env.as_deref(),
            Some(&environment),
            is_test_build,
        );

        Self {
            database_url,
            host,
            port,
            electric_url,
            electric_secret,
            oidc,
            dev_auth_enabled,
            environment,
        }
    }

    /// 設定値の整合性検証 (本番環境での OIDC 設定必須化など)
    pub fn validate(&self) -> Result<(), String> {
        if self.environment == "production" && self.oidc.is_none() {
            return Err(
                "OIDC configuration (OIDC_ISSUER and OIDC_AUDIENCE) is required when ENVIRONMENT is 'production'"
                    .to_string(),
            );
        }
        Ok(())
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
            electric_url: "http://127.0.0.1:3000".to_string(),
            electric_secret: Some("secret123".to_string()),
            oidc: Some(OidcConfig {
                issuer: "https://idp.example.com".to_string(),
                audience: "zoo-client".to_string(),
                jwks_url: Some("https://idp.example.com/jwks".to_string()),
            }),
            dev_auth_enabled: false,
            environment: "production".to_string(),
        };
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("super_secret_password"));
        assert!(!debug_str.contains("secret123"));
        assert!(!debug_str.contains("user:"));
        assert!(debug_str.contains("[REDACTED]"));
        assert!(debug_str.contains("8080"));
        assert!(debug_str.contains("https://idp.example.com"));
    }

    #[test]
    fn test_config_validate_in_production() {
        // 本番環境で OIDC 設定なしはエラー
        let prod_no_oidc = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: None,
            dev_auth_enabled: false,
            environment: "production".to_string(),
        };
        assert!(prod_no_oidc.validate().is_err());

        // 開発環境なら OIDC 設定なしでも OK
        let dev_no_oidc = Config {
            environment: "development".to_string(),
            ..prod_no_oidc.clone()
        };
        assert!(dev_no_oidc.validate().is_ok());

        // 本番環境でも OIDC 設定があれば OK
        let prod_with_oidc = Config {
            oidc: Some(OidcConfig {
                issuer: "https://idp.example.com".to_string(),
                audience: "zoo-client".to_string(),
                jwks_url: None,
            }),
            ..prod_no_oidc
        };
        assert!(prod_with_oidc.validate().is_ok());
    }
}
