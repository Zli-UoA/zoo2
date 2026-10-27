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

/// 開発用の既定 CORS 許可オリジン
///
/// dev はフロント (`:5173`) からバックエンド (`:8000`) へのクロスオリジンになるため、
/// localhost / 127.0.0.1 の両表記を既定で許可する。本番は同一オリジン配信のため
/// 外部オリジンは不要で、`CORS_ALLOWED_ORIGINS` で明示的に上書きする。
pub(crate) const DEFAULT_CORS_ALLOWED_ORIGINS: &[&str] =
    &["http://localhost:5173", "http://127.0.0.1:5173"];

/// `CORS_ALLOWED_ORIGINS` (カンマ区切り) をパースする純粋関数
///
/// - 設定済みならカンマ区切りをパースする (空要素は無視、空文字なら空リスト)。
/// - 未設定なら dev 用の既定オリジンを返す。ただし `ENVIRONMENT=production` では
///   本番に dev オリジン (`localhost:5173` / `127.0.0.1:5173`) を残さないため
///   空リストを既定とする (本番は同一オリジン配信で外部オリジン不要)。
pub(crate) fn parse_cors_allowed_origins(raw: Option<&str>, environment: &str) -> Vec<String> {
    match raw {
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect(),
        None if environment == "production" => Vec::new(),
        None => DEFAULT_CORS_ALLOWED_ORIGINS
            .iter()
            .map(|origin| origin.to_string())
            .collect(),
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
    pub cors_allowed_origins: Vec<String>,
    pub webrtc_ice_servers_json: String,
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
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("webrtc_ice_servers_json", &self.webrtc_ice_servers_json)
            .finish()
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// OIDC 環境変数から `OidcConfig` を組み立てる純粋関数
///
/// 空文字は未設定として扱い、`OIDC_AUDIENCE` が空なら `OIDC_CLIENT_ID` に
/// フォールバックする。`OIDC_ISSUER` と audience の両方が揃わなければ `None`。
pub(crate) fn parse_oidc_config(
    issuer: Option<&str>,
    audience: Option<&str>,
    client_id: Option<&str>,
    jwks_url: Option<&str>,
) -> Option<OidcConfig> {
    let issuer = non_empty(issuer)?;
    let audience = non_empty(audience).or_else(|| non_empty(client_id))?;
    let jwks_url = non_empty(jwks_url).map(str::to_string);

    Some(OidcConfig {
        issuer: issuer.to_string(),
        audience: audience.to_string(),
        jwks_url,
    })
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

        let oidc = parse_oidc_config(
            env::var("OIDC_ISSUER").ok().as_deref(),
            env::var("OIDC_AUDIENCE").ok().as_deref(),
            env::var("OIDC_CLIENT_ID").ok().as_deref(),
            env::var("OIDC_JWKS_URL").ok().as_deref(),
        );

        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());

        let cors_allowed_origins = parse_cors_allowed_origins(
            env::var("CORS_ALLOWED_ORIGINS").ok().as_deref(),
            &environment,
        );

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

        let webrtc_ice_servers_json = env::var("WEBRTC_ICE_SERVERS_JSON")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| r#"[{"urls":["stun:stun.l.google.com:19302"]}]"#.to_string());

        Self {
            database_url,
            host,
            port,
            electric_url,
            electric_secret,
            oidc,
            dev_auth_enabled,
            environment,
            cors_allowed_origins,
            webrtc_ice_servers_json,
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
            cors_allowed_origins: vec!["https://zoo.example.com".to_string()],
            webrtc_ice_servers_json: r#"[{"urls":["stun:stun.l.google.com:19302"]}]"#.to_string(),
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
            cors_allowed_origins: vec![],
            webrtc_ice_servers_json: r#"[{"urls":["stun:stun.l.google.com:19302"]}]"#.to_string(),
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

    #[test]
    fn test_parse_oidc_config_requires_issuer_and_audience() {
        assert!(parse_oidc_config(None, Some("zoo-client"), None, None).is_none());
        assert!(parse_oidc_config(Some("https://idp.example.com"), None, None, None).is_none());

        let oidc = parse_oidc_config(
            Some("https://idp.example.com"),
            Some("zoo-client"),
            None,
            Some("https://idp.example.com/jwks"),
        )
        .expect("issuer and audience should produce OIDC config");
        assert_eq!(oidc.issuer, "https://idp.example.com");
        assert_eq!(oidc.audience, "zoo-client");
        assert_eq!(
            oidc.jwks_url.as_deref(),
            Some("https://idp.example.com/jwks")
        );
    }

    #[test]
    fn test_parse_oidc_config_treats_empty_values_as_unset() {
        // 空文字は設定済みとして扱わない。
        assert!(parse_oidc_config(Some(""), Some("zoo-client"), None, None).is_none());
        assert!(parse_oidc_config(Some("https://idp.example.com"), Some(""), None, None).is_none());
        assert!(
            parse_oidc_config(Some(""), Some(""), Some(""), Some("")).is_none(),
            "all-empty OIDC variables must be treated as unset"
        );

        // 空の audience は client_id にフォールバックする。
        let oidc = parse_oidc_config(
            Some("https://idp.example.com"),
            Some(""),
            Some("zoo-client"),
            Some(""),
        )
        .expect("client_id should be used when audience is empty");
        assert_eq!(oidc.audience, "zoo-client");
        assert_eq!(oidc.jwks_url, None, "empty jwks_url must be unset");
    }

    #[test]
    fn test_production_validate_rejects_empty_oidc_values() {
        // compose が空文字を渡しても from_env と同じ組み立て結果になり、本番では起動できない。
        let config = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: parse_oidc_config(Some(""), Some(""), Some(""), Some("")),
            dev_auth_enabled: false,
            environment: "production".to_string(),
            cors_allowed_origins: vec![],
            webrtc_ice_servers_json: r#"[{"urls":["stun:stun.l.google.com:19302"]}]"#.to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_parse_cors_allowed_origins_defaults_to_dev() {
        // development で未設定なら dev オリジンを含む
        let origins = parse_cors_allowed_origins(None, "development");
        assert!(origins.contains(&"http://localhost:5173".to_string()));
        assert!(origins.contains(&"http://127.0.0.1:5173".to_string()));
        assert_eq!(origins.len(), DEFAULT_CORS_ALLOWED_ORIGINS.len());
    }

    #[test]
    fn test_parse_cors_allowed_origins_defaults_empty_in_production() {
        // 本番で未設定なら dev オリジンを残さない (空リスト)
        assert!(parse_cors_allowed_origins(None, "production").is_empty());
    }

    #[test]
    fn test_parse_cors_allowed_origins_from_env() {
        // 設定済みなら環境に関わらずパースされる (本番でも明示オリジンは許可)
        let origins = parse_cors_allowed_origins(
            Some("https://zoo.example.com, https://app.example.com "),
            "production",
        );
        assert_eq!(
            origins,
            vec![
                "https://zoo.example.com".to_string(),
                "https://app.example.com".to_string(),
            ]
        );

        // カンマ区切り・前後空白を除去
        let origins = parse_cors_allowed_origins(
            Some("https://zoo.example.com, https://app.example.com "),
            "development",
        );
        assert_eq!(
            origins,
            vec![
                "https://zoo.example.com".to_string(),
                "https://app.example.com".to_string(),
            ]
        );

        // 空要素は無視
        let origins =
            parse_cors_allowed_origins(Some("https://zoo.example.com,, ,"), "development");
        assert_eq!(origins, vec!["https://zoo.example.com".to_string()]);

        // 空文字はクロスオリジン無効 (空リスト)
        assert!(parse_cors_allowed_origins(Some(""), "development").is_empty());
        assert!(parse_cors_allowed_origins(Some(""), "production").is_empty());
    }

    #[test]
    fn test_webrtc_ice_servers_default_and_debug() {
        let config = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: None,
            dev_auth_enabled: false,
            environment: "development".to_string(),
            cors_allowed_origins: vec![],
            webrtc_ice_servers_json: r#"[{"urls":["stun:stun.l.google.com:19302"]}]"#.to_string(),
        };
        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("stun:stun.l.google.com:19302"));
    }
}
