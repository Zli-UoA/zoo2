use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{config::OidcConfig, error::AppError};

/// OpenID Provider Configuration (.well-known/openid-configuration)
#[derive(Debug, Deserialize)]
struct OpenIdConfiguration {
    jwks_uri: String,
}

/// OIDC トークンのクレーム定義 (RFC 9068 適合)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcClaims {
    pub sub: String,
    pub iss: String,
    pub aud: serde_json::Value,
    pub exp: u64,
    pub nbf: Option<u64>,
    pub iat: Option<u64>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub scope: Option<serde_json::Value>,
}

/// JWKS キャッシュ
struct JwksCache {
    keys: HashMap<String, (DecodingKey, Algorithm)>,
    last_fetched_at: Option<Instant>,
}

/// JWKS によるトークン検証マネージャ
#[derive(Clone)]
pub struct JwksVerifier {
    cache: Arc<RwLock<JwksCache>>,
    http_client: reqwest::Client,
    cache_ttl: Duration,
    min_fetch_interval: Duration,
}

impl JwksVerifier {
    pub fn new(http_client: reqwest::Client) -> Self {
        Self {
            cache: Arc::new(RwLock::new(JwksCache {
                keys: HashMap::new(),
                last_fetched_at: None,
            })),
            http_client,
            cache_ttl: Duration::from_secs(24 * 3600), // 24時間キャッシュ
            min_fetch_interval: Duration::from_secs(5), // 最低取得間隔5秒 (DoS防止)
        }
    }

    /// テスト用など、直接キーを追加したキャッシュを構築
    pub fn with_keys(keys: HashMap<String, (DecodingKey, Algorithm)>) -> Self {
        Self {
            cache: Arc::new(RwLock::new(JwksCache {
                keys,
                last_fetched_at: Some(Instant::now()),
            })),
            http_client: reqwest::Client::new(),
            cache_ttl: Duration::from_secs(24 * 3600),
            min_fetch_interval: Duration::from_secs(0),
        }
    }

    /// JWKS URL の解決 (明示設定がある場合はそれを使い、未設定の場合は OpenID Discovery を使用)
    async fn resolve_jwks_url(&self, config: &OidcConfig) -> Result<String, AppError> {
        if let Some(ref url) = config.jwks_url {
            return Ok(url.clone());
        }

        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            config.issuer.trim_end_matches('/')
        );

        let resp = self
            .http_client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("Failed to fetch OpenID discovery from {discovery_url}: {e}");
                AppError::Unauthorized("Failed to verify token".to_string())
            })?;

        if !resp.status().is_success() {
            tracing::warn!(
                "OpenID discovery endpoint {discovery_url} returned status {}",
                resp.status()
            );
            return Err(AppError::Unauthorized("Failed to verify token".to_string()));
        }

        let openid_config: OpenIdConfiguration = resp.json().await.map_err(|e| {
            tracing::warn!("Failed to parse OpenID discovery JSON: {e}");
            AppError::Unauthorized("Failed to verify token".to_string())
        })?;

        Ok(openid_config.jwks_uri)
    }

    /// JWKS をエンドポイントから取得してキャッシュを更新
    async fn refresh_jwks(&self, jwks_url: &str) -> Result<(), AppError> {
        let resp = self.http_client.get(jwks_url).send().await.map_err(|e| {
            tracing::warn!("Failed to fetch JWKS from {jwks_url}: {e}");
            AppError::Unauthorized("Failed to verify token".to_string())
        })?;

        if !resp.status().is_success() {
            tracing::warn!("JWKS endpoint {jwks_url} returned status {}", resp.status());
            return Err(AppError::Unauthorized("Failed to verify token".to_string()));
        }

        let jwks: JwkSet = resp.json().await.map_err(|e| {
            tracing::warn!("Failed to parse JWKS JSON: {e}");
            AppError::Unauthorized("Failed to verify token".to_string())
        })?;

        let mut new_keys = HashMap::new();
        for key in jwks.keys {
            if let Some(ref kid) = key.common.key_id {
                let alg = match key.common.key_algorithm {
                    Some(jsonwebtoken::jwk::KeyAlgorithm::RS256) => Algorithm::RS256,
                    Some(jsonwebtoken::jwk::KeyAlgorithm::ES256) => Algorithm::ES256,
                    Some(jsonwebtoken::jwk::KeyAlgorithm::RS384) => Algorithm::RS384,
                    Some(jsonwebtoken::jwk::KeyAlgorithm::ES384) => Algorithm::ES384,
                    Some(jsonwebtoken::jwk::KeyAlgorithm::RS512) => Algorithm::RS512,
                    _ => Algorithm::RS256,
                };
                if let Ok(decoding_key) = DecodingKey::from_jwk(&key) {
                    new_keys.insert(kid.clone(), (decoding_key, alg));
                }
            }
        }

        let mut cache = self.cache.write().await;
        cache.keys = new_keys;
        cache.last_fetched_at = Some(Instant::now());

        Ok(())
    }

    /// Bearer トークンを検証し、クレームを返却 (RFC 9068 準拠)
    pub async fn verify_token(
        &self,
        token: &str,
        config: &OidcConfig,
    ) -> Result<OidcClaims, AppError> {
        // 1. ヘッダのデコード
        let header = decode_header(token).map_err(|e| {
            tracing::warn!("Failed to decode token header: {e}");
            AppError::Unauthorized("Invalid token header".to_string())
        })?;

        // 2. RFC 9068 Section 2.1 による typ: at+jwt の検証 (IDトークンとの混同防止)
        let is_valid_typ = header
            .typ
            .as_deref()
            .map(|t| {
                t.eq_ignore_ascii_case("at+jwt") || t.eq_ignore_ascii_case("application/at+jwt")
            })
            .unwrap_or(false);

        if !is_valid_typ {
            tracing::warn!(
                "Token rejected: header typ is {:?}, required 'at+jwt' (RFC 9068)",
                header.typ
            );
            return Err(AppError::Unauthorized(
                "Invalid token type (RFC 9068 at+jwt required)".to_string(),
            ));
        }

        let kid = header.kid.ok_or_else(|| {
            tracing::warn!("Token header is missing 'kid'");
            AppError::Unauthorized("Invalid token header".to_string())
        })?;

        // 3. キャッシュからキーを検索 (TTL超過時は再取得)
        let cached = {
            let cache = self.cache.read().await;
            let is_expired = match cache.last_fetched_at {
                Some(t) => t.elapsed() >= self.cache_ttl,
                None => true,
            };
            if is_expired {
                None
            } else {
                cache.keys.get(&kid).cloned()
            }
        };

        let (decoding_key, alg) = match cached {
            Some(key_pair) => key_pair,
            None => {
                let need_refresh = {
                    let cache = self.cache.read().await;
                    match cache.last_fetched_at {
                        Some(t) => t.elapsed() >= self.min_fetch_interval,
                        None => true,
                    }
                };

                if need_refresh {
                    let jwks_url = self.resolve_jwks_url(config).await?;
                    self.refresh_jwks(&jwks_url).await?;
                }

                let cache = self.cache.read().await;
                cache.keys.get(&kid).cloned().ok_or_else(|| {
                    tracing::warn!("Key id (kid) '{kid}' not found in JWKS");
                    AppError::Unauthorized("Invalid token signature".to_string())
                })?
            }
        };

        // 4. Validation 設定 (RFC 9068, iss, aud, exp, nbf の検証)
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&[&config.audience]);
        validation.validate_exp = true;
        validation.validate_nbf = true;

        // 5. トークンのデコードと署名・期限検証
        let token_data = decode::<OidcClaims>(token, &decoding_key, &validation).map_err(|e| {
            tracing::warn!("Token validation failed: {e}");
            AppError::Unauthorized("Invalid token".to_string())
        })?;

        Ok(token_data.claims)
    }
}
