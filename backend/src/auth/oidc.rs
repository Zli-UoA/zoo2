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
    pub issuer: String,
    pub jwks_uri: String,
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
    #[serde(alias = "tid", alias = "tenant")]
    pub tenant_id: Option<String>,
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
                AppError::BadGateway("Failed to connect to OpenID discovery endpoint".to_string())
            })?;

        if !resp.status().is_success() {
            tracing::warn!(
                "OpenID discovery endpoint {discovery_url} returned status {}",
                resp.status()
            );
            return Err(AppError::BadGateway(format!(
                "OpenID discovery endpoint returned status {}",
                resp.status()
            )));
        }

        let openid_config: OpenIdConfiguration = resp.json().await.map_err(|e| {
            tracing::warn!("Failed to parse OpenID discovery JSON: {e}");
            AppError::BadGateway("Failed to parse OpenID discovery JSON".to_string())
        })?;

        // OIDC Discovery 仕様に基づく issuer の照合 (末尾スラッシュの差異を許容)
        if openid_config.issuer.trim_end_matches('/') != config.issuer.trim_end_matches('/') {
            tracing::warn!(
                "OpenID discovery issuer mismatch: expected '{}', got '{}'",
                config.issuer,
                openid_config.issuer
            );
            return Err(AppError::BadGateway(
                "OpenID discovery issuer mismatch".to_string(),
            ));
        }

        Ok(openid_config.jwks_uri)
    }

    /// JWKS をエンドポイントから取得してキャッシュを更新
    async fn refresh_jwks(&self, jwks_url: &str) -> Result<(), AppError> {
        let resp = self.http_client.get(jwks_url).send().await.map_err(|e| {
            tracing::warn!("Failed to fetch JWKS from {jwks_url}: {e}");
            AppError::BadGateway("Failed to fetch JWKS from identity provider".to_string())
        })?;

        if !resp.status().is_success() {
            tracing::warn!("JWKS endpoint {jwks_url} returned status {}", resp.status());
            return Err(AppError::BadGateway(format!(
                "JWKS endpoint returned status {}",
                resp.status()
            )));
        }

        let jwks: JwkSet = resp.json().await.map_err(|e| {
            tracing::warn!("Failed to parse JWKS JSON: {e}");
            AppError::BadGateway("Failed to parse JWKS response from identity provider".to_string())
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
                    _ => match key.algorithm {
                        // Kanidm の既定は ES256 (alg 省略時も kty/parameters から推論)
                        jsonwebtoken::jwk::AlgorithmParameters::EllipticCurve(_) => {
                            Algorithm::ES256
                        }
                        jsonwebtoken::jwk::AlgorithmParameters::RSA(_) => Algorithm::RS256,
                        _ => Algorithm::RS256,
                    },
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

    /// トークンのデコードと検証処理
    fn decode_and_validate(
        &self,
        token: &str,
        decoding_key: &DecodingKey,
        alg: Algorithm,
        config: &OidcConfig,
    ) -> Result<OidcClaims, AppError> {
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&[&config.audience]);
        validation.validate_exp = true;
        validation.validate_nbf = true;

        let token_data = decode::<OidcClaims>(token, decoding_key, &validation).map_err(|e| {
            tracing::warn!("Token validation failed: {e}");
            AppError::Unauthorized("Invalid token".to_string())
        })?;

        Ok(token_data.claims)
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

        // 3. キャッシュからキーを検索 (TTL超過時は再取得、失敗時は stale キャッシュへフォールバック)
        let (cached, is_expired) = {
            let cache = self.cache.read().await;
            let expired = match cache.last_fetched_at {
                Some(t) => t.elapsed() >= self.cache_ttl,
                None => true,
            };
            (cache.keys.get(&kid).cloned(), expired)
        };

        let (decoding_key, alg) = match cached {
            Some(key_pair) if !is_expired => key_pair,
            cached_stale => {
                let need_refresh = {
                    let cache = self.cache.read().await;
                    match cache.last_fetched_at {
                        Some(t) => t.elapsed() >= self.min_fetch_interval,
                        None => true,
                    }
                };

                if need_refresh {
                    match self.resolve_jwks_url(config).await {
                        Ok(jwks_url) => {
                            if let Err(err) = self.refresh_jwks(&jwks_url).await {
                                tracing::warn!("JWKS refresh failed: {err}");
                                if let Some(stale) = cached_stale {
                                    tracing::warn!("Falling back to stale JWKS key for kid: {kid}");
                                    return self
                                        .decode_and_validate(token, &stale.0, stale.1, config);
                                }
                                return Err(err);
                            }
                        }
                        Err(err) => {
                            tracing::warn!("OpenID Discovery failed: {err}");
                            if let Some(stale) = cached_stale {
                                tracing::warn!("Falling back to stale JWKS key for kid: {kid}");
                                return self.decode_and_validate(token, &stale.0, stale.1, config);
                            }
                            return Err(err);
                        }
                    }
                }

                let cache = self.cache.read().await;
                cache.keys.get(&kid).cloned().ok_or_else(|| {
                    tracing::warn!("Key id (kid) '{kid}' not found in JWKS");
                    AppError::Unauthorized("Invalid token signature".to_string())
                })?
            }
        };

        // 4. トークンのデコードと検証
        self.decode_and_validate(token, &decoding_key, alg, config)
    }
}
