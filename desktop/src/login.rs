//! OIDC デバイスコード認証・トークン管理と開発モード (`X-Dev-User`) 接続。

use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc::UnboundedSender;
use zoo2_client_core::{
    api::{ApiClient, Me},
    auth::{
        self, Discovery, TokenResult,
        device::{self, Poller},
        token::{self, StoredTokens, TokenManager},
    },
    config::{self, AuthConfig, Config, OidcConfig},
    http::{BearerSource, Credentials, HttpError},
};

pub const APP_NAME: &str = "zoo2-desktop";

/// 期限までの余裕秒数 (`client-core` の `REFRESH_SKEW_SECONDS` と同じ)。
const REFRESH_SKEW_SECONDS: i64 = 60;

/// 同一プロセス内で `token.json` の読み書き・削除を直列化するロック。
static TOKEN_FILE_LOCK: Mutex<()> = Mutex::new(());

/// セッション単位でトークンの更新・永続化・ログアウト状態を一元管理するマネージャ。
///
/// - 認証タスク完了後、UI 側で現行セッション世代 (`session_gen`) と照合して採用されるまで
///   `can_persist` は `false` のため、キャンセルされた古いログインタスクが `token.json` を上書きしない。
/// - ログアウト時は `TOKEN_FILE_LOCK` 下で `token.json` を削除すると同時に `logged_out = true` にするため、
///   進行中だった古いリフレッシュ応答が遅れて返ってきても `token.json` を再生成・上書きしない。
pub struct DesktopTokenManager {
    http: reqwest::Client,
    oidc: OidcConfig,
    path: PathBuf,
    logged_out: AtomicBool,
    can_persist: AtomicBool,
    tokens_sync: Mutex<StoredTokens>,
    refresh_lock: tokio::sync::Mutex<Option<Discovery>>,
}

impl DesktopTokenManager {
    pub fn new(
        http: reqwest::Client,
        oidc: OidcConfig,
        path: PathBuf,
        tokens: StoredTokens,
        can_persist: bool,
    ) -> Self {
        Self {
            http,
            oidc,
            path,
            logged_out: AtomicBool::new(false),
            can_persist: AtomicBool::new(can_persist),
            tokens_sync: Mutex::new(tokens),
            refresh_lock: tokio::sync::Mutex::new(None),
        }
    }

    /// 現行セッションとして採用された時点で呼び出し、`token.json` への保存を有効化して書き込む。
    pub fn enable_persistence_and_save(&self) -> Result<()> {
        let _guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if self.logged_out.load(Ordering::SeqCst) {
            bail!("ログアウト済みのトークンです");
        }
        let tokens = self
            .tokens_sync
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        token::save(&self.path, &tokens)?;
        self.can_persist.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// このセッションからの `token.json` への書き込みを停止する。
    pub fn disable_persistence(&self) {
        let _guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.can_persist.store(false, Ordering::SeqCst);
    }

    /// ローカルの `token.json` を削除し、成功した場合のみこのマネージャをログアウト済みにする。
    /// 削除に失敗した場合は状態を変えず `Err` を返す。
    pub fn logout_and_delete(&self) -> Result<Option<String>> {
        let _guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        token::delete(&self.path)?;
        self.logged_out.store(true, Ordering::SeqCst);
        self.can_persist.store(false, Ordering::SeqCst);
        let refresh_token = self
            .tokens_sync
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .refresh_token
            .clone();
        Ok(refresh_token)
    }

    pub async fn access_token(&self) -> Result<String> {
        if self.logged_out.load(Ordering::SeqCst) {
            return Err(HttpError::Unauthorized.into());
        }
        let mut discovery_guard = self.refresh_lock.lock().await;
        if self.logged_out.load(Ordering::SeqCst) {
            return Err(HttpError::Unauthorized.into());
        }
        let (current_access, expires_at, refresh_token, api_base_url) = {
            let tokens = self.tokens_sync.lock().unwrap_or_else(|e| e.into_inner());
            (
                tokens.access_token.clone(),
                tokens.expires_at,
                tokens.refresh_token.clone(),
                tokens.api_base_url.clone(),
            )
        };
        if expires_at - REFRESH_SKEW_SECONDS > token::now() {
            return Ok(current_access);
        }
        let Some(refresh_token) = refresh_token else {
            return Err(HttpError::Unauthorized.into());
        };
        if discovery_guard.is_none() {
            *discovery_guard = Some(auth::discover(&self.http, &self.oidc.issuer).await?);
        }
        let discovery = discovery_guard.as_ref().expect("直前に設定した");
        let form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
        ];
        let response = match auth::token_request(&self.http, discovery, &self.oidc, form).await? {
            TokenResult::Ok(response) => response,
            TokenResult::Err { error, .. } if needs_login(&error) => {
                return Err(HttpError::Unauthorized.into());
            }
            TokenResult::Err { error, description } => {
                return Err(anyhow!(
                    "トークンを更新できませんでした: {}",
                    description.unwrap_or(error)
                ));
            }
        };
        let mut tokens = StoredTokens::new(&self.oidc, &api_base_url, response, token::now());
        tokens.refresh_token = tokens.refresh_token.or(Some(refresh_token));

        let _file_guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if self.logged_out.load(Ordering::SeqCst) {
            return Err(HttpError::Unauthorized.into());
        }
        *self.tokens_sync.lock().unwrap_or_else(|e| e.into_inner()) = tokens.clone();
        if self.can_persist.load(Ordering::SeqCst) {
            token::save(&self.path, &tokens)?;
        }
        Ok(tokens.access_token)
    }
}

fn needs_login(error: &str) -> bool {
    matches!(
        error,
        "invalid_grant" | "invalid_client" | "unauthorized_client" | "unsupported_grant_type"
    )
}

impl BearerSource for DesktopTokenManager {
    fn bearer(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(self.access_token())
    }
}

/// ログイン画面の状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginPhase {
    /// 起動時または再試行時の接続・トークン確認中。
    Checking,
    /// ログイン操作待ち (エラーメッセージがある場合は表示する)。
    Ready { error: Option<String> },
    /// デバイスコードを発行し、ブラウザでのユーザー承認を待っている状態。
    AwaitingApproval {
        verification_uri: String,
        user_code: String,
    },
}

/// バックグラウンド認証タスクから UI へ送るイベント。
#[derive(Clone)]
pub enum AuthEvent {
    /// 認証・ユーザー情報取得に成功し、チャット画面へ遷移できる。
    Connected {
        credentials: Credentials,
        token_manager: Option<Arc<DesktopTokenManager>>,
        me: Me,
    },
    /// デバイスコードが発行されたので画面に表示する。
    DevicePrompt {
        verification_uri: String,
        user_code: String,
    },
    /// ログインが必要、または接続に失敗した。
    LoginRequired { error: Option<String> },
}

impl std::fmt::Debug for AuthEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected { me, .. } => f.debug_struct("Connected").field("me", me).finish(),
            Self::DevicePrompt {
                verification_uri,
                user_code,
            } => f
                .debug_struct("DevicePrompt")
                .field("verification_uri", verification_uri)
                .field("user_code", user_code)
                .finish(),
            Self::LoginRequired { error } => f
                .debug_struct("LoginRequired")
                .field("error", error)
                .finish(),
        }
    }
}

pub fn token_file() -> Result<PathBuf> {
    config::config_dir_for(APP_NAME)
        .map(|dir| token::token_path(&dir))
        .ok_or_else(|| anyhow!("設定ディレクトリが見つかりません"))
}

fn oidc_config(config: &Config) -> Result<&OidcConfig> {
    match &config.auth {
        AuthConfig::Oidc(oidc) => Ok(oidc),
        AuthConfig::Dev { .. } => {
            bail!(
                "dev モードでは OIDC ログインを使えません。ZOO_DEV_USER と dev_user を外してください"
            )
        }
    }
}

/// 起動時の認証チェック。Dev モードならそのまま `/api/users/me` を呼び、
/// OIDC モードなら保存済みトークンを読み込んで有効性を確かめる。
pub async fn connect_initial(
    http: reqwest::Client,
    config: Config,
    tx: UnboundedSender<AuthEvent>,
) {
    let (credentials, token_manager) = match &config.auth {
        AuthConfig::Dev { user_id } => (Credentials::Dev(user_id.clone()), None),
        AuthConfig::Oidc(oidc) => {
            let path = match token_file() {
                Ok(path) => path,
                Err(err) => {
                    let _ = tx.send(AuthEvent::LoginRequired {
                        error: Some(format!("{err:#}")),
                    });
                    return;
                }
            };
            let loaded = {
                let _guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                token::load(&path, oidc, Some(&config.api_base_url))
            };
            match loaded {
                Ok(Some(tokens)) => {
                    let manager = Arc::new(DesktopTokenManager::new(
                        http.clone(),
                        oidc.clone(),
                        path,
                        tokens,
                        false,
                    ));
                    (Credentials::Bearer(manager.clone()), Some(manager))
                }
                Ok(None) => {
                    let _ = tx.send(AuthEvent::LoginRequired { error: None });
                    return;
                }
                Err(err) => {
                    let _ = tx.send(AuthEvent::LoginRequired {
                        error: Some(format!("保存済みトークンの読み込みに失敗しました: {err:#}")),
                    });
                    return;
                }
            }
        }
    };

    let api = ApiClient::new(http, &config.api_base_url, credentials.clone());
    match api.me().await {
        Ok(me) => {
            let _ = tx.send(AuthEvent::Connected {
                credentials,
                token_manager,
                me,
            });
        }
        Err(err) => {
            let message = if matches!(
                err.downcast_ref::<HttpError>(),
                Some(HttpError::Unauthorized)
            ) {
                "セッションの有効期限が切れました。再度ログインしてください".to_string()
            } else {
                format!("接続に失敗しました: {err:#}")
            };
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(message),
            });
        }
    }
}

/// GUI のログイン画面からデバイスコードフローを実行する。
pub async fn run_device_login(
    http: reqwest::Client,
    config: Config,
    tx: UnboundedSender<AuthEvent>,
) {
    let oidc = match oidc_config(&config) {
        Ok(oidc) => oidc,
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("{err:#}")),
            });
            return;
        }
    };

    let discovery = match auth::discover(&http, &oidc.issuer).await {
        Ok(d) => d,
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("OIDC メタデータの取得に失敗しました: {err:#}")),
            });
            return;
        }
    };

    let authorization = match device::start(&http, &discovery, oidc).await {
        Ok(a) => a,
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("デバイスコードの取得に失敗しました: {err:#}")),
            });
            return;
        }
    };

    let url = authorization
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| authorization.verification_uri.clone());

    open_browser(&url);
    let _ = tx.send(AuthEvent::DevicePrompt {
        verification_uri: url,
        user_code: authorization.user_code.clone(),
    });

    let response = match Poller::default()
        .poll(&http, &discovery, oidc, &authorization)
        .await
    {
        Ok(res) => res,
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("ログイン承認に失敗しました: {err:#}")),
            });
            return;
        }
    };

    let tokens = StoredTokens::new(oidc, &config.api_base_url, response, token::now());
    let path = match token_file() {
        Ok(p) => p,
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("{err:#}")),
            });
            return;
        }
    };

    // トークンファイルへの保存は UI 側で現行世代と確認された後に行う (`can_persist = false`)。
    let manager = Arc::new(DesktopTokenManager::new(
        http.clone(),
        oidc.clone(),
        path,
        tokens,
        false,
    ));
    let credentials = Credentials::Bearer(manager.clone());
    let api = ApiClient::new(http, &config.api_base_url, credentials.clone());
    match api
        .me()
        .await
        .context("ログインはできましたが、zoo2 がトークンを受け付けませんでした")
    {
        Ok(me) => {
            let _ = tx.send(AuthEvent::Connected {
                credentials,
                token_manager: Some(manager),
                me,
            });
        }
        Err(err) => {
            let _ = tx.send(AuthEvent::LoginRequired {
                error: Some(format!("{err:#}")),
            });
        }
    }
}

/// ローカルのトークンファイルを即座に削除し、失効対象のリフレッシュトークンを返す。
pub fn delete_local_token_for_logout(config: &Config) -> Result<(bool, Option<String>)> {
    let oidc = oidc_config(config)?;
    let path = token_file()?;
    let _guard = TOKEN_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let refresh_token = token::load(&path, oidc, None)
        .ok()
        .flatten()
        .and_then(|tokens| tokens.refresh_token);
    let deleted = token::delete(&path)?;
    Ok((deleted, refresh_token))
}

/// IdP の revocation エンドポイントへリフレッシュトークンの失効を要求する。
pub async fn revoke_refresh_token(http: reqwest::Client, oidc: OidcConfig, refresh_token: String) {
    if let Ok(discovery) = auth::discover(&http, &oidc.issuer).await
        && let Some(endpoint) = discovery.revocation_endpoint
    {
        let form = [
            ("token", refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
            ("client_id", oidc.client_id.as_str()),
        ];
        let _ = http.post(endpoint).form(&form).send().await;
    }
}

/// IdP のリフレッシュトークンを失効させ、ローカルのトークンファイルを削除する。
pub async fn logout_session(http: reqwest::Client, config: Config) -> Result<bool> {
    let oidc = oidc_config(&config)?.clone();
    let (deleted, refresh_token) = delete_local_token_for_logout(&config)?;
    if let Some(refresh_token) = refresh_token {
        revoke_refresh_token(http, oidc, refresh_token).await;
    }
    Ok(deleted)
}

/// CLI から `zoo2-desktop login` を実行する。
pub async fn cli_login(config: &Config) -> Result<()> {
    let oidc = oidc_config(config)?;
    let http = reqwest::Client::new();
    let discovery = auth::discover(&http, &oidc.issuer).await?;
    let authorization = device::start(&http, &discovery, oidc).await?;

    let url = authorization
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&authorization.verification_uri);
    println!("ブラウザで次の URL を開き、コードを確かめて承認してください。");
    println!();
    println!("  URL:   {url}");
    println!("  コード: {}", authorization.user_code);
    println!();
    open_browser(url);
    println!("承認を待っています… (Ctrl+C で中止)");

    let response = Poller::default()
        .poll(&http, &discovery, oidc, &authorization)
        .await?;
    let tokens = StoredTokens::new(oidc, &config.api_base_url, response, token::now());
    let path = token_file()?;
    token::save(&path, &tokens)?;

    let manager = TokenManager::new(http.clone(), oidc.clone(), path, tokens);
    let api = ApiClient::new(
        http,
        &config.api_base_url,
        Credentials::Bearer(Arc::new(manager)),
    );
    let me = api
        .me()
        .await
        .context("ログインはできましたが、zoo2 がトークンを受け付けませんでした")?;
    println!(
        "{} (@{}) としてログインしました。",
        me.display_name, me.user_name
    );
    Ok(())
}

/// CLI から `zoo2-desktop logout` を実行する。
pub async fn cli_logout(config: &Config) -> Result<()> {
    let http = reqwest::Client::new();
    if logout_session(http, config.clone()).await? {
        println!("ログアウトしました。");
    } else {
        println!("ログインしていません。");
    }
    Ok(())
}

/// HTTP / HTTPS の有効な URL かどうかを検証し、正規化済みの `reqwest::Url` を返す。
pub fn validate_http_url(url: &str) -> Option<reqwest::Url> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str()?;
    Some(parsed)
}

/// OS の既定ブラウザで URL を開く (macOS / Windows / Linux 対応)。
/// Windows では `cmd.exe` を経由せず `ShellExecuteW` を直接呼び出し、`&` 等によるコマンド実行を防ぐ。
pub fn open_browser(url: &str) -> bool {
    let Some(parsed) = validate_http_url(url) else {
        return false;
    };
    let safe_url = parsed.as_str();
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(safe_url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "windows")]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut core::ffi::c_void,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }

        const SW_SHOWNORMAL: i32 = 1;
        let operation: Vec<u16> = OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let file: Vec<u16> = OsStr::new(safe_url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `operation` と `file` は NUL 終端された有効な UTF-16 スライスであり、省略可能な引数は null ポインタを渡している。
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        result > 32
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::process::Command::new("xdg-open")
            .arg(safe_url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_file_points_to_zoo2_desktop_config_dir() {
        if let Ok(path) = token_file() {
            let text = path.to_string_lossy();
            assert!(text.contains("zoo2-desktop"));
            assert!(text.ends_with("token.json"));
        }
    }

    #[test]
    fn validates_http_and_https_urls_only() {
        assert!(validate_http_url("https://example.com/?a=1&b=2").is_some());
        assert!(validate_http_url("http://localhost:8000/api").is_some());
        assert!(validate_http_url("javascript:alert(1)").is_none());
        assert!(validate_http_url("file:///etc/passwd").is_none());
        assert!(validate_http_url("not a url").is_none());
    }

    #[test]
    fn desktop_token_manager_isolates_sessions_and_prevents_post_logout_writes() {
        let dir = std::env::temp_dir().join(format!(
            "zoo2-desktop-token-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "zoo2-desktop".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        };
        let tokens1 = StoredTokens {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: "https://api.example".into(),
            access_token: "access-1".into(),
            refresh_token: Some("refresh-1".into()),
            expires_at: token::now() + 300,
        };
        let tm1 = DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            path.clone(),
            tokens1,
            false,
        );
        // 採用前はファイルに書き込まれない
        assert!(!path.exists());
        tm1.enable_persistence_and_save().unwrap();
        assert!(path.exists());

        // ログアウトするとファイルが消え、旧マネージャからの再保存は拒否される
        let rt = tm1.logout_and_delete().unwrap();
        assert_eq!(rt.as_deref(), Some("refresh-1"));
        assert!(!path.exists());
        assert!(tm1.enable_persistence_and_save().is_err());
        assert!(!path.exists());

        // 新しいセッションが保存したトークンは旧マネージャに影響されない
        let tokens2 = StoredTokens {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: "https://api.example".into(),
            access_token: "access-2".into(),
            refresh_token: Some("refresh-2".into()),
            expires_at: token::now() + 300,
        };
        let tm2 = DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            path.clone(),
            tokens2,
            false,
        );
        tm2.enable_persistence_and_save().unwrap();
        let loaded = token::load(&path, &oidc, Some("https://api.example"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.access_token, "access-2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
