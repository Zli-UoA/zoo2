use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{config::OidcConfig, error::AppError};

/// ユーザー区分
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Affiliation {
    Freshman,
    Student,
    Alumni,
    Unknown,
}

impl Affiliation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Freshman => "freshman",
            Self::Student => "student",
            Self::Alumni => "alumni",
            Self::Unknown => "unknown",
        }
    }
}

impl From<&str> for Affiliation {
    fn from(s: &str) -> Self {
        match s {
            "freshman" => Self::Freshman,
            "student" => Self::Student,
            "alumni" => Self::Alumni,
            _ => Self::Unknown,
        }
    }
}

/// OIDC トークンのクレーム定義
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
    /// scope はスペース区切りの文字列または文字列配列
    pub scope: Option<serde_json::Value>,
}

impl OidcClaims {
    /// scope をトークンから抽出してリスト化
    pub fn extract_scopes(&self) -> Vec<String> {
        match &self.scope {
            Some(serde_json::Value::String(s)) => {
                s.split_whitespace().map(|x| x.to_string()).collect()
            }
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// OidcConfig に基づき Affiliation を解決
    pub fn resolve_affiliation(&self, config: &OidcConfig) -> Affiliation {
        let scopes = self.extract_scopes();

        if scopes.iter().any(|s| s == &config.scope_student) {
            Affiliation::Student
        } else if scopes.iter().any(|s| s == &config.scope_freshman) {
            Affiliation::Freshman
        } else if scopes.iter().any(|s| s == &config.scope_alumni) {
            Affiliation::Alumni
        } else {
            Affiliation::Unknown
        }
    }
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

    /// JWKS をエンドポイントから取得してキャッシュを更新
    async fn refresh_jwks(&self, jwks_url: &str) -> Result<(), AppError> {
        let resp = self
            .http_client
            .get(jwks_url)
            .send()
            .await
            .map_err(|e| AppError::Unauthorized(format!("Failed to fetch JWKS: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::Unauthorized(format!(
                "JWKS endpoint returned status {}",
                resp.status()
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Unauthorized(format!("Failed to read JWKS body: {e}")))?;

        let jwks: JwkSet = serde_json::from_str(&body)
            .map_err(|e| AppError::Unauthorized(format!("Failed to parse JWKS JSON: {e}")))?;

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

    /// Bearer トークンを検証し、クレームとユーザー区分を返却
    pub async fn verify_token(
        &self,
        token: &str,
        config: &OidcConfig,
    ) -> Result<(OidcClaims, Affiliation), AppError> {
        // 1. ヘッダのデコード
        let header = decode_header(token)
            .map_err(|e| AppError::Unauthorized(format!("Invalid token header: {e}")))?;

        let kid = header
            .kid
            .ok_or_else(|| AppError::Unauthorized("Token header is missing 'kid'".to_string()))?;

        // 2. キャッシュからキーを検索 (TTL超過時は再取得)
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

        let (decoding_key, alg) =
            match cached {
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
                        self.refresh_jwks(&config.jwks_url).await?;
                    }

                    let cache = self.cache.read().await;
                    cache.keys.get(&kid).cloned().ok_or_else(|| {
                        AppError::Unauthorized(format!("Unknown key id (kid): {kid}"))
                    })?
                }
            };

        // 3. Validation 設定
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&[&config.audience]);
        validation.validate_exp = true;

        // 4. トークンのデコードと署名・期限検証
        let token_data = decode::<OidcClaims>(token, &decoding_key, &validation)
            .map_err(|e| AppError::Unauthorized(format!("Token validation failed: {e}")))?;

        let claims = token_data.claims;
        let affiliation = claims.resolve_affiliation(config);

        Ok((claims, affiliation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affiliation_resolution() {
        let config = OidcConfig {
            issuer: "https://idp.example.com".to_string(),
            audience: "zoo-app".to_string(),
            jwks_url: "https://idp.example.com/jwks".to_string(),
            scope_student: "student".to_string(),
            scope_freshman: "freshman".to_string(),
            scope_alumni: "alumni".to_string(),
        };

        // 1. Student
        let claims = OidcClaims {
            sub: "user-1".into(),
            iss: "https://idp.example.com".into(),
            aud: serde_json::json!("zoo-app"),
            exp: 9999999999,
            nbf: None,
            iat: None,
            preferred_username: Some("alice".into()),
            name: Some("Alice".into()),
            email: None,
            scope: Some(serde_json::json!("openid profile student")),
        };
        assert_eq!(claims.resolve_affiliation(&config), Affiliation::Student);

        // 2. Freshman
        let claims = OidcClaims {
            scope: Some(serde_json::json!(["openid", "freshman"])),
            ..claims.clone()
        };
        assert_eq!(claims.resolve_affiliation(&config), Affiliation::Freshman);

        // 3. Alumni
        let claims = OidcClaims {
            scope: Some(serde_json::json!("alumni")),
            ..claims.clone()
        };
        assert_eq!(claims.resolve_affiliation(&config), Affiliation::Alumni);

        // 4. Unknown
        let claims = OidcClaims {
            scope: Some(serde_json::json!("openid other")),
            ..claims.clone()
        };
        assert_eq!(claims.resolve_affiliation(&config), Affiliation::Unknown);
    }
}
