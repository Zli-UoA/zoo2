use std::{env, fmt};

#[derive(Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub scope_student: String,
    pub scope_freshman: String,
    pub scope_alumni: String,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("jwks_url", &self.jwks_url)
            .field("scope_student", &self.scope_student)
            .field("scope_freshman", &self.scope_freshman)
            .field("scope_alumni", &self.scope_alumni)
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
                let jwks_url = env::var("OIDC_JWKS_URL").unwrap_or_else(|_| {
                    format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/'))
                });
                let scope_student =
                    env::var("OIDC_SCOPE_STUDENT").unwrap_or_else(|_| "student".to_string());
                let scope_freshman =
                    env::var("OIDC_SCOPE_FRESHMAN").unwrap_or_else(|_| "freshman".to_string());
                let scope_alumni =
                    env::var("OIDC_SCOPE_ALUMNI").unwrap_or_else(|_| "alumni".to_string());

                Some(OidcConfig {
                    issuer,
                    audience,
                    jwks_url,
                    scope_student,
                    scope_freshman,
                    scope_alumni,
                })
            }
            _ => None,
        };

        Self {
            database_url,
            host,
            port,
            electric_url,
            electric_secret,
            oidc,
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
            electric_url: "http://127.0.0.1:3000".to_string(),
            electric_secret: Some("secret123".to_string()),
            oidc: Some(OidcConfig {
                issuer: "https://idp.example.com".to_string(),
                audience: "zoo-client".to_string(),
                jwks_url: "https://idp.example.com/jwks".to_string(),
                scope_student: "student".to_string(),
                scope_freshman: "freshman".to_string(),
                scope_alumni: "alumni".to_string(),
            }),
        };
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("super_secret_password"));
        assert!(!debug_str.contains("secret123"));
        assert!(!debug_str.contains("user:"));
        assert!(debug_str.contains("[REDACTED]"));
        assert!(debug_str.contains("8080"));
        assert!(debug_str.contains("https://idp.example.com"));
    }
}
