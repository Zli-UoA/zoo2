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

/// S3 公開エンドポイント環境変数（`S3_PUBLIC_ENDPOINT`）をパース・正規化する純粋関数
///
/// 未設定または空文字なら `None` を返す。
/// 指定されている場合は前後空白を除去し、末尾のスラッシュを削除して返す。
pub(crate) fn parse_public_endpoint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
}

#[derive(Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    pub presigned_put_expiry_secs: u64,
    pub presigned_get_expiry_secs: u64,
    pub max_file_size: u64,
    pub public_endpoint: Option<String>,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field("force_path_style", &self.force_path_style)
            .field("presigned_put_expiry_secs", &self.presigned_put_expiry_secs)
            .field("presigned_get_expiry_secs", &self.presigned_get_expiry_secs)
            .field("max_file_size", &self.max_file_size)
            .field("public_endpoint", &self.public_endpoint)
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
    pub cors_allowed_origins: Vec<String>,
    pub s3: S3Config,
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
            .field("s3", &self.s3)
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

        let s3_endpoint =
            env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
        let s3_bucket = env::var("S3_BUCKET").unwrap_or_else(|_| "zoo-attachments-dev".to_string());
        let s3_region = env::var("S3_REGION").unwrap_or_else(|_| "auto".to_string());
        let s3_access_key_id =
            env::var("S3_ACCESS_KEY_ID").unwrap_or_else(|_| "rustfsadmin".to_string());
        let s3_secret_access_key =
            env::var("S3_SECRET_ACCESS_KEY").unwrap_or_else(|_| "rustfsadmin".to_string());
        let s3_force_path_style = env::var("S3_FORCE_PATH_STYLE")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(true);
        let presigned_put_expiry_secs = env::var("ATTACHMENT_PRESIGNED_PUT_EXPIRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900);
        let presigned_get_expiry_secs = env::var("ATTACHMENT_PRESIGNED_GET_EXPIRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        let max_file_size = env::var("ATTACHMENT_MAX_FILE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(52428800);
        let public_endpoint = parse_public_endpoint(env::var("S3_PUBLIC_ENDPOINT").ok().as_deref());

        let s3 = S3Config {
            endpoint: s3_endpoint,
            bucket: s3_bucket,
            region: s3_region,
            access_key_id: s3_access_key_id,
            secret_access_key: s3_secret_access_key,
            force_path_style: s3_force_path_style,
            presigned_put_expiry_secs,
            presigned_get_expiry_secs,
            max_file_size,
            public_endpoint,
        };

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
            s3,
        }
    }

    /// 設定値の整合性検証 (本番環境での OIDC 設定必須化および S3/R2 設定必須化など)
    pub fn validate(&self) -> Result<(), String> {
        const MAX_PRESIGNED_EXPIRY_SECS: u64 = 604_800; // AWS SigV4 上限 (7 日)
        if !(1..=MAX_PRESIGNED_EXPIRY_SECS).contains(&self.s3.presigned_put_expiry_secs) {
            return Err(format!(
                "ATTACHMENT_PRESIGNED_PUT_EXPIRY_SECS must be between 1 and {MAX_PRESIGNED_EXPIRY_SECS} seconds, got {}",
                self.s3.presigned_put_expiry_secs
            ));
        }
        if !(1..=MAX_PRESIGNED_EXPIRY_SECS).contains(&self.s3.presigned_get_expiry_secs) {
            return Err(format!(
                "ATTACHMENT_PRESIGNED_GET_EXPIRY_SECS must be between 1 and {MAX_PRESIGNED_EXPIRY_SECS} seconds, got {}",
                self.s3.presigned_get_expiry_secs
            ));
        }

        if let Some(ref public_endpoint) = self.s3.public_endpoint {
            let parsed = reqwest::Url::parse(public_endpoint)
                .map_err(|e| format!("S3_PUBLIC_ENDPOINT is not a valid URL: {e}"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err("S3_PUBLIC_ENDPOINT scheme must be http or https".to_string());
            }
            if self.environment == "production" && parsed.scheme() != "https" {
                return Err("S3_PUBLIC_ENDPOINT must use https in production".to_string());
            }
            if parsed.host_str().is_none() {
                return Err("S3_PUBLIC_ENDPOINT must have a host".to_string());
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err("S3_PUBLIC_ENDPOINT must not contain userinfo".to_string());
            }
            if parsed.query().is_some() {
                return Err("S3_PUBLIC_ENDPOINT must not contain query parameters".to_string());
            }
            if parsed.fragment().is_some() {
                return Err("S3_PUBLIC_ENDPOINT must not contain a fragment".to_string());
            }
            if !self.s3.force_path_style {
                return Err(
                    "S3_PUBLIC_ENDPOINT requires S3_FORCE_PATH_STYLE=true (path-style URLs)"
                        .to_string(),
                );
            }
        }

        if self.environment == "production" {
            if self.oidc.is_none() {
                return Err(
                    "OIDC configuration (OIDC_ISSUER and OIDC_AUDIENCE) is required when ENVIRONMENT is 'production'"
                        .to_string(),
                );
            }

            // 本番環境における S3 / Cloudflare R2 設定の検証
            if self.s3.endpoint.is_empty()
                || self.s3.endpoint == "http://localhost:9000"
                || self.s3.endpoint == "http://127.0.0.1:9000"
            {
                return Err(
                    "S3_ENDPOINT must be explicitly set to a production storage endpoint (e.g. Cloudflare R2) when ENVIRONMENT is 'production'"
                        .to_string(),
                );
            }
            if self.s3.bucket.is_empty() || self.s3.bucket == "zoo-attachments-dev" {
                return Err(
                    "S3_BUCKET must be explicitly set to a production bucket name when ENVIRONMENT is 'production'"
                        .to_string(),
                );
            }
            if self.s3.access_key_id.is_empty() || self.s3.access_key_id == "rustfsadmin" {
                return Err(
                    "S3_ACCESS_KEY_ID must be set and cannot use the development default 'rustfsadmin' when ENVIRONMENT is 'production'"
                        .to_string(),
                );
            }
            if self.s3.secret_access_key.is_empty() || self.s3.secret_access_key == "rustfsadmin" {
                return Err(
                    "S3_SECRET_ACCESS_KEY must be set and cannot use the development default 'rustfsadmin' when ENVIRONMENT is 'production'"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_s3_config() -> S3Config {
        S3Config {
            endpoint: "https://example.r2.cloudflarestorage.com".to_string(),
            bucket: "zoo-attachments-prod".to_string(),
            region: "auto".to_string(),
            access_key_id: "prod_key_id".to_string(),
            secret_access_key: "super_secret_s3_key".to_string(),
            force_path_style: false,
            presigned_put_expiry_secs: 900,
            presigned_get_expiry_secs: 300,
            max_file_size: 52428800,
            public_endpoint: None,
        }
    }

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
            s3: dummy_s3_config(),
        };
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("super_secret_password"));
        assert!(!debug_str.contains("secret123"));
        assert!(!debug_str.contains("super_secret_s3_key"));
        assert!(!debug_str.contains("prod_key_id"));
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
            s3: dummy_s3_config(),
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
            s3: dummy_s3_config(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_production_validate_rejects_default_or_missing_s3_config() {
        let valid_oidc = parse_oidc_config(
            Some("https://idp.example.com"),
            Some("zoo-client"),
            None,
            Some("https://idp.example.com/jwks"),
        );

        // 1. 正常な本番設定 (OIDC あり、本番 R2 設定あり)
        let mut config = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: valid_oidc.clone(),
            dev_auth_enabled: false,
            environment: "production".to_string(),
            cors_allowed_origins: vec![],
            s3: dummy_s3_config(),
        };
        assert!(config.validate().is_ok());

        // 2. 開発用 localhost endpoint のまま
        config.s3.endpoint = "http://localhost:9000".to_string();
        assert!(config.validate().is_err());

        // 3. 開発用バケット名のまま
        config.s3 = dummy_s3_config();
        config.s3.bucket = "zoo-attachments-dev".to_string();
        assert!(config.validate().is_err());

        // 4. 開発用デフォルトクレデンシャル (rustfsadmin) のまま
        config.s3 = dummy_s3_config();
        config.s3.access_key_id = "rustfsadmin".to_string();
        assert!(config.validate().is_err());

        config.s3 = dummy_s3_config();
        config.s3.secret_access_key = "rustfsadmin".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_presigned_expiry_bounds() {
        let mut config = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: None,
            dev_auth_enabled: true,
            environment: "development".to_string(),
            cors_allowed_origins: vec![],
            s3: dummy_s3_config(),
        };

        // 境界値 OK (1 秒、604,800 秒 = 7 日)
        config.s3.presigned_put_expiry_secs = 1;
        config.s3.presigned_get_expiry_secs = 604_800;
        assert!(config.validate().is_ok());

        config.s3.presigned_put_expiry_secs = 604_800;
        config.s3.presigned_get_expiry_secs = 1;
        assert!(config.validate().is_ok());

        // 範囲外 NG (0 秒、604,801 秒)
        config.s3.presigned_put_expiry_secs = 0;
        config.s3.presigned_get_expiry_secs = 300;
        assert!(config.validate().is_err());

        config.s3.presigned_put_expiry_secs = 604_801;
        config.s3.presigned_get_expiry_secs = 300;
        assert!(config.validate().is_err());

        config.s3.presigned_put_expiry_secs = 900;
        config.s3.presigned_get_expiry_secs = 0;
        assert!(config.validate().is_err());

        config.s3.presigned_put_expiry_secs = 900;
        config.s3.presigned_get_expiry_secs = 604_801;
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
    fn test_parse_public_endpoint() {
        assert_eq!(parse_public_endpoint(None), None);
        assert_eq!(parse_public_endpoint(Some("")), None);
        assert_eq!(parse_public_endpoint(Some("   ")), None);

        // トリムおよび末尾スラッシュの除去
        assert_eq!(
            parse_public_endpoint(Some("https://attachments.zliusercontent.com")),
            Some("https://attachments.zliusercontent.com".to_string())
        );
        assert_eq!(
            parse_public_endpoint(Some("  https://attachments.zliusercontent.com/  ")),
            Some("https://attachments.zliusercontent.com".to_string())
        );
        assert_eq!(
            parse_public_endpoint(Some("https://attachments.zliusercontent.com///")),
            Some("https://attachments.zliusercontent.com".to_string())
        );
    }

    #[test]
    fn test_validate_public_endpoint() {
        let valid_oidc = Some(OidcConfig {
            issuer: "https://idm.example.com".to_string(),
            audience: "zoo_client".to_string(),
            jwks_url: None,
        });

        let mut config = Config {
            database_url: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            electric_url: "test".to_string(),
            electric_secret: None,
            oidc: valid_oidc.clone(),
            dev_auth_enabled: false,
            environment: "development".to_string(),
            cors_allowed_origins: vec![],
            s3: dummy_s3_config(),
        };

        // None は OK
        config.s3.public_endpoint = None;
        assert!(config.validate().is_ok());

        // public_endpoint がある場合は force_path_style = true が必須
        config.s3.public_endpoint = Some("https://attachments.zliusercontent.com".to_string());
        config.s3.force_path_style = false;
        assert!(config.validate().is_err());

        config.s3.force_path_style = true;
        assert!(config.validate().is_ok());

        // dev では http / https 両方 OK
        config.s3.public_endpoint = Some("http://localhost:8787".to_string());
        assert!(config.validate().is_ok());

        config.s3.public_endpoint = Some("https://attachments.zliusercontent.com".to_string());
        assert!(config.validate().is_ok());

        // 不正な URL
        config.s3.public_endpoint = Some("not-a-url".to_string());
        assert!(config.validate().is_err());

        // 不正なスキーム (ftp://)
        config.s3.public_endpoint = Some("ftp://example.com".to_string());
        assert!(config.validate().is_err());

        // userinfo (ユーザー名・パスワード) の混入拒否
        config.s3.public_endpoint =
            Some("https://user:pass@attachments.zliusercontent.com".to_string());
        assert!(config.validate().is_err());

        // クエリパラメータの混入拒否
        config.s3.public_endpoint =
            Some("https://attachments.zliusercontent.com?foo=bar".to_string());
        assert!(config.validate().is_err());

        // フラグメントの混入拒否
        config.s3.public_endpoint =
            Some("https://attachments.zliusercontent.com#section".to_string());
        assert!(config.validate().is_err());

        // 本番環境 (production)
        config.environment = "production".to_string();
        config.s3.public_endpoint = Some("https://attachments.zliusercontent.com".to_string());
        assert!(config.validate().is_ok());

        // 本番環境での http は拒否
        config.s3.public_endpoint = Some("http://attachments.zliusercontent.com".to_string());
        assert!(config.validate().is_err());
    }
}
