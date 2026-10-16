use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::RwLock;

use crate::{
    auth::{AuthUser, oidc::JwksVerifier},
    config::Config,
};

type UserCacheMap = HashMap<(String, String), (AuthUser, Instant)>;

/// 認証済みユーザーのインメモリキャッシュ (JIT 解決結果のキャッシュ)
#[derive(Clone, Default)]
pub struct UserResolutionCache {
    entries: Arc<RwLock<UserCacheMap>>,
}

impl UserResolutionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, iss: &str, sub: &str) -> Option<AuthUser> {
        let cache = self.entries.read().await;
        let key = (iss.to_string(), sub.to_string());
        if let Some((user, expires_at)) = cache.get(&key)
            && Instant::now() < *expires_at
        {
            return Some(user.clone());
        }
        None
    }

    pub async fn insert(&self, iss: String, sub: String, user: AuthUser, ttl: Duration) {
        let mut cache = self.entries.write().await;
        if cache.len() > 10000 {
            let now = Instant::now();
            cache.retain(|_, (_, exp)| *exp > now);
        }
        let expires_at = Instant::now() + ttl;
        cache.insert((iss, sub), (user, expires_at));
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    pub config: Config,
    pub http_client: reqwest::Client,
    pub jwks_verifier: JwksVerifier,
    pub user_cache: UserResolutionCache,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();

        Self {
            db,
            config,
            http_client,
            jwks_verifier,
            user_cache,
        }
    }

    pub fn with_client(db: toasty::Db, config: Config, http_client: reqwest::Client) -> Self {
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();
        Self {
            db,
            config,
            http_client,
            jwks_verifier,
            user_cache,
        }
    }

    pub fn with_verifier(
        db: toasty::Db,
        config: Config,
        http_client: reqwest::Client,
        jwks_verifier: JwksVerifier,
    ) -> Self {
        let user_cache = UserResolutionCache::new();
        Self {
            db,
            config,
            http_client,
            jwks_verifier,
            user_cache,
        }
    }
}
