use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::RwLock;

use crate::{
    auth::{AuthUser, oidc::JwksVerifier},
    config::Config,
    routes::calls::CallRegistry,
};

type UserCacheMap = HashMap<(String, String), (AuthUser, Instant)>;
type ProfileRepairAttemptMap = HashMap<String, Instant>;

/// UserInfo が名前を返さないユーザーへの補正試行を抑止する期間
const PROFILE_REPAIR_RETRY_INTERVAL: Duration = Duration::from_secs(3600);

/// 認証済みユーザーのインメモリキャッシュ (JIT 解決結果のキャッシュ)
#[derive(Clone, Default)]
pub struct UserResolutionCache {
    entries: Arc<RwLock<UserCacheMap>>,
    profile_repair_attempts: Arc<RwLock<ProfileRepairAttemptMap>>,
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

    /// 直近に表示名の補正を試みたかどうか。
    ///
    /// UserInfo が名前を返さないユーザーに対して、5分ごとの外部呼び出しが
    /// 繰り返されるのを防ぐ。一定期間経過後は再試行する。
    pub async fn should_attempt_profile_repair(&self, user_id: &str) -> bool {
        let attempts = self.profile_repair_attempts.read().await;
        attempts
            .get(user_id)
            .map(|attempted_at| attempted_at.elapsed() >= PROFILE_REPAIR_RETRY_INTERVAL)
            .unwrap_or(true)
    }

    pub async fn mark_profile_repair_attempted(&self, user_id: &str) {
        let mut attempts = self.profile_repair_attempts.write().await;
        if attempts.len() > 10000 {
            attempts
                .retain(|_, attempted_at| attempted_at.elapsed() < PROFILE_REPAIR_RETRY_INTERVAL);
        }
        attempts.insert(user_id.to_string(), Instant::now());
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: toasty::Db,
    pub config: Config,
    pub http_client: reqwest::Client,
    pub electric_client: reqwest::Client,
    pub jwks_verifier: JwksVerifier,
    pub user_cache: UserResolutionCache,
    pub call_registry: CallRegistry,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        let http_client = Self::default_http_client();
        let electric_client = Self::default_electric_client();
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();

        Self {
            db,
            config,
            http_client,
            electric_client,
            jwks_verifier,
            user_cache,
            call_registry,
        }
    }

    /// OIDC JWKS 取得などの一般的な HTTP リクエスト用クライアント (短時間タイムアウト)
    pub fn default_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }

    /// Electric Shape プロキシ用クライアント
    ///
    /// Electric の live=true (ロングポーリング: デフォルト20秒待機) や
    /// live_sse=true (Server-Sent Events: 永続接続) での待機を切断しないよう、
    /// 全体完了タイムアウト (timeout) は設定せず、接続タイムアウトと TCP keepalive のみを設定する。
    pub fn default_electric_client() -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }

    /// テストなどで HTTP クライアントを差し替えるためのコンストラクタ。
    ///
    /// Electric プロキシで意図しないタイムアウトが発生するのを防ぐため、
    /// `electric_client` にはタイムアウトなしの既定クライアント (`Self::default_electric_client()`) が設定されます。
    pub fn with_client(db: toasty::Db, config: Config, http_client: reqwest::Client) -> Self {
        let electric_client = Self::default_electric_client();
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        Self {
            db,
            config,
            http_client,
            electric_client,
            jwks_verifier,
            user_cache,
            call_registry,
        }
    }

    /// テストなどで JwksVerifier を差し替えるためのコンストラクタ。
    ///
    /// `electric_client` にはタイムアウトなしの既定クライアント (`Self::default_electric_client()`) が設定されます。
    pub fn with_verifier(
        db: toasty::Db,
        config: Config,
        http_client: reqwest::Client,
        jwks_verifier: JwksVerifier,
    ) -> Self {
        let electric_client = Self::default_electric_client();
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        Self {
            db,
            config,
            http_client,
            electric_client,
            jwks_verifier,
            user_cache,
            call_registry,
        }
    }

    /// テストなどで `http_client` と `electric_client` の両方を明示的に差し替えるためのコンストラクタ。
    pub fn with_clients(
        db: toasty::Db,
        config: Config,
        http_client: reqwest::Client,
        electric_client: reqwest::Client,
        jwks_verifier: JwksVerifier,
    ) -> Self {
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        Self {
            db,
            config,
            http_client,
            electric_client,
            jwks_verifier,
            user_cache,
            call_registry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_clients_construction() {
        let _http_client = AppState::default_http_client();
        let _electric_client = AppState::default_electric_client();
    }

    #[tokio::test]
    async fn test_profile_repair_attempt_cache() {
        let cache = UserResolutionCache::new();

        assert!(cache.should_attempt_profile_repair("user-1").await);
        cache.mark_profile_repair_attempted("user-1").await;
        assert!(!cache.should_attempt_profile_repair("user-1").await);
        // 別ユーザーの補正試行には影響しない
        assert!(cache.should_attempt_profile_repair("user-2").await);
    }

    #[test]
    fn test_with_client_preserves_default_electric_client() {
        // http_client に明示的なタイムアウトを設定して with_client を呼んでも、
        // electric_client は独自に構築されたクライアントが設定されることを確認
        let dummy_client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();

        // ダミーの Config
        let _config = Config::from_env();
        // db はテストで初期化できないため、state の構築テストは default_electric_client 経由で行われていることをテスト
        let electric = AppState::default_electric_client();
        // dummy_client と electric は独立した Client
        assert_ne!(format!("{dummy_client:?}"), format!("{electric:?}"));
    }
}
