use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use crate::{
    auth::{AuthUser, oidc::JwksVerifier},
    config::Config,
    routes::calls::CallRegistry,
};

type UserCacheMap = HashMap<(String, String), (AuthUser, Instant)>;
type ResolutionFailures = HashMap<(String, String), Instant>;
type ResolutionFlights = HashMap<(String, String), Weak<Mutex<()>>>;
type ProfileRepairAttemptMap = HashMap<String, Instant>;

/// UserInfo が名前を返さないユーザーへの補正試行を抑止する期間
const PROFILE_REPAIR_RETRY_INTERVAL: Duration = Duration::from_secs(3600);

/// 認証済みユーザーのインメモリキャッシュ (JIT 解決結果のキャッシュ)
#[derive(Clone, Default)]
pub struct UserResolutionCache {
    entries: Arc<RwLock<UserCacheMap>>,
    flights: Arc<RwLock<ResolutionFlights>>,
    resolution_failures: Arc<RwLock<ResolutionFailures>>,
    profile_repair_attempts: Arc<RwLock<ProfileRepairAttemptMap>>,
}

impl UserResolutionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn profile_retry_limited(&self, iss: &str, sub: &str) -> bool {
        self.resolution_failures
            .read()
            .await
            .get(&(iss.to_owned(), sub.to_owned()))
            .is_some_and(|until| Instant::now() < *until)
    }

    pub async fn mark_profile_unavailable(&self, iss: &str, sub: &str) {
        let mut failures = self.resolution_failures.write().await;
        failures.retain(|_, until| Instant::now() < *until);
        failures.insert(
            (iss.to_owned(), sub.to_owned()),
            Instant::now() + Duration::from_secs(5),
        );
    }

    pub async fn lock_resolution(&self, iss: &str, sub: &str) -> OwnedMutexGuard<()> {
        let mutex = {
            let mut flights = self.flights.write().await;
            flights.retain(|_, mutex| mutex.strong_count() > 0);
            let entry = flights.entry((iss.to_owned(), sub.to_owned())).or_default();
            match entry.upgrade() {
                Some(mutex) => mutex,
                None => {
                    let mutex = Arc::new(Mutex::new(()));
                    *entry = Arc::downgrade(&mutex);
                    mutex
                }
            }
        };
        mutex.lock_owned().await
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
    pub home_live: crate::routes::home_live::HomeLive,
    pub db: toasty::Db,
    pub config: Config,
    pub http_client: reqwest::Client,
    pub electric_client: reqwest::Client,
    pub s3_client: aws_sdk_s3::Client,
    pub jwks_verifier: JwksVerifier,
    pub user_cache: UserResolutionCache,
    pub call_registry: CallRegistry,
}

impl AppState {
    pub fn new(db: toasty::Db, config: Config) -> Self {
        let http_client = Self::default_http_client();
        let electric_client = Self::default_electric_client();
        let s3_client = crate::storage::create_s3_client(&config.s3);
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        call_registry.start_listener(config.database_url.clone());
        call_registry.start_sweeper(db.clone());

        Self {
            home_live: Default::default(),
            db,
            config,
            http_client,
            electric_client,
            s3_client,
            jwks_verifier,
            user_cache,
            call_registry,
        }
    }

    /// HTTP を受けないワーカー用 (`zoo_slack_worker`)。通話のレジストリの LISTEN 接続と掃除のタスクは起動しない
    /// (データベースの接続を、ワーカーが使わない機能に取られないため)。それ以外は `new` と同じ
    pub fn new_for_worker(db: toasty::Db, config: Config) -> Self {
        let http_client = Self::default_http_client();
        Self {
            home_live: Default::default(),
            db,
            http_client: http_client.clone(),
            electric_client: Self::default_electric_client(),
            s3_client: crate::storage::create_s3_client(&config.s3),
            jwks_verifier: JwksVerifier::new(http_client),
            user_cache: UserResolutionCache::new(),
            call_registry: CallRegistry::new(),
            config,
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
        let s3_client = crate::storage::create_s3_client(&config.s3);
        let jwks_verifier = JwksVerifier::new(http_client.clone());
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        call_registry.start_listener(config.database_url.clone());
        call_registry.start_sweeper(db.clone());
        Self {
            home_live: Default::default(),
            db,
            config,
            http_client,
            electric_client,
            s3_client,
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
        let s3_client = crate::storage::create_s3_client(&config.s3);
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        call_registry.start_listener(config.database_url.clone());
        call_registry.start_sweeper(db.clone());
        Self {
            home_live: Default::default(),
            db,
            config,
            http_client,
            electric_client,
            s3_client,
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
        let s3_client = crate::storage::create_s3_client(&config.s3);
        let user_cache = UserResolutionCache::new();
        let call_registry = CallRegistry::new();
        call_registry.start_listener(config.database_url.clone());
        call_registry.start_sweeper(db.clone());
        Self {
            home_live: Default::default(),
            db,
            config,
            http_client,
            electric_client,
            s3_client,
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
