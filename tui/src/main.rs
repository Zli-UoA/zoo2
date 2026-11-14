use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use zoo2_tui::{
    api::ApiClient,
    auth::{
        self,
        device::{self, Poller},
        token::{self, StoredTokens, TokenManager},
    },
    config::{self, AuthConfig, Config, OidcConfig},
    e2ee,
    http::Credentials,
    ui,
};

const USAGE: &str = "\
使い方: zoo2-tui [コマンド]

コマンド:
  run      チャット画面を開く (省略時の既定)
  login    ブラウザでコードを承認してログインする
  logout   この端末の暗号の鍵と、保存したトークンを消す
  help     このヘルプを表示する

設定は ~/.config/zoo2-tui/config.toml と環境変数 ZOO_* から読み込みます。";

#[tokio::main]
async fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());
    match command.as_str() {
        "run" => run().await,
        "login" => login().await,
        "logout" => logout().await,
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("不明なコマンドです: {other}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

fn oidc_config(config: &Config) -> Result<&OidcConfig> {
    match &config.auth {
        AuthConfig::Oidc(oidc) => Ok(oidc),
        AuthConfig::Dev { .. } => {
            bail!("dev モードではログインできません。ZOO_DEV_USER と dev_user を外してください")
        }
    }
}

fn token_file() -> Result<PathBuf> {
    config::config_dir()
        .map(|dir| token::token_path(&dir))
        .ok_or_else(|| anyhow!("設定ディレクトリが見つかりません"))
}

async fn run() -> Result<()> {
    let config = Config::load()?;
    let http = reqwest::Client::new();
    let credentials = match &config.auth {
        AuthConfig::Dev { user_id } => Credentials::Dev(user_id.clone()),
        AuthConfig::Oidc(oidc) => {
            let path = token_file()?;
            let tokens = token::load_async(&path, oidc, Some(&config.api_base_url))
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "ログインしていません。先に `zoo2-tui login` を実行してください。\
                     接続先や OIDC の設定を変えたときも、ログインし直してください"
                    )
                })?;
            Credentials::Bearer(Arc::new(TokenManager::new(
                http.clone(),
                oidc.clone(),
                path,
                tokens,
            )))
        }
    };
    // 端末の登録と to-device の処理は、画面と並行して裏で行う（#359）。
    let crypto_root = config::config_dir().map(|dir| e2ee::store::crypto_root(&dir));
    let crypto = config::config_dir().map(|dir| {
        let user_id = match &credentials {
            Credentials::Dev(user_id) => Some(user_id.clone()),
            Credentials::Bearer(_) => None,
        };
        e2ee::manager::spawn(
            http.clone(),
            config.api_base_url.clone(),
            credentials.clone(),
            ApiClient::new(http.clone(), &config.api_base_url, credentials.clone()),
            e2ee::store::crypto_root(&dir),
            user_id,
        )
    });
    let (crypto_task, e2ee) = match crypto {
        Some((task, stores)) => {
            // 初回設定と解錠の再認証（#360）。dev モードではサーバーも再認証を確かめない。
            let oidc = match &config.auth {
                AuthConfig::Oidc(oidc) => Some(oidc.clone()),
                AuthConfig::Dev { .. } => None,
            };
            (
                Some(task),
                Some(ui::encryption::E2eeContext {
                    oidc,
                    stores,
                    crypto_root,
                }),
            )
        }
        None => (None, None),
    };
    let result = ui::run(
        http,
        config.api_base_url,
        credentials,
        config.check_updates,
        e2ee,
    )
    .await;
    if let Some(task) = crypto_task {
        task.abort();
    }
    result
}

async fn login() -> Result<()> {
    let config = Config::load()?;
    let oidc = oidc_config(&config)?;
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
    token::save_async(&path, &tokens).await?;

    // backend がこのトークンを受け付けるか (resource と audience が合っているか) を確かめる。
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

async fn logout() -> Result<()> {
    let config = Config::load()?;
    let http = reqwest::Client::new();
    // dev モードでも、run で作った暗号ストアと端末は消す。トークンの失効はない。
    if let AuthConfig::Dev { user_id } = &config.auth {
        let credentials = Credentials::Dev(user_id.clone());
        if !clear_encryption(&config, credentials, &http).await? {
            println!("ログアウトを取りやめました。");
        } else {
            println!("この端末の暗号の鍵を消しました。");
        }
        return Ok(());
    }
    let oidc = oidc_config(&config)?;
    let path = token_file()?;
    // 端末の削除 API にはトークンが要る。トークンがなければ、次のログインで消し直す。
    let credentials = match token::load(&path, oidc, Some(&config.api_base_url))? {
        Some(tokens) => Credentials::Bearer(Arc::new(TokenManager::new(
            http.clone(),
            oidc.clone(),
            path.clone(),
            tokens,
        ))),
        None => Credentials::Dev(String::new()),
    };
    if !clear_encryption(&config, credentials, &http).await? {
        println!("ログアウトを取りやめました。");
        return Ok(());
    }
    match revoke_and_delete_tokens(&path, oidc, &http).await? {
        token::DeleteOutcome::Deleted => println!("ログアウトしました。"),
        token::DeleteOutcome::Missing => println!("ログインしていません。"),
        token::DeleteOutcome::Changed => {
            println!("ログアウト中にログイン状態が変わったため、新しいログインを保持しました。")
        }
    }
    Ok(())
}

/// 暗号ストアを消し、端末を削除する（docs/e2ee.md の「ログアウトと端末の削除」）。
///
/// 鍵バックアップに送れていない鍵があれば、件数を示して確かめる。取りやめたら `false`。
async fn clear_encryption(
    config: &Config,
    credentials: Credentials,
    http: &reqwest::Client,
) -> Result<bool> {
    let Some(dir) = config::config_dir() else {
        return Ok(true);
    };
    let root = e2ee::store::crypto_root(&dir);
    let targets = e2ee::manager::open_for_logout(&root)?;
    if targets.is_empty() {
        return Ok(true);
    }
    let pending: usize = targets.iter().map(|target| target.pending_backup).sum();
    if pending > 0 {
        println!(
            "{pending} 件の鍵がバックアップされていません。ログアウトすると、この端末でしか復号できない過去のメッセージが読めなくなります。"
        );
        print!("ログアウトしますか？ [y/N] ");
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(false);
        }
    }
    let api = ApiClient::new(http.clone(), &config.api_base_url, credentials);
    // 削除 API は、トークンのユーザーの暗号ストアにだけ呼ぶ。分からなければ（オフラインなど）、
    // 削除待ちに残して次の起動で消し直す。
    let signed_in_user = api.me().await.ok().map(|me| me.id);
    for target in targets {
        e2ee::manager::destroy_for_logout(&api, &root, target, signed_in_user.as_deref()).await?;
    }
    Ok(true)
}

async fn revoke_and_delete_tokens(
    path: &Path,
    oidc: &OidcConfig,
    http: &reqwest::Client,
) -> Result<token::DeleteOutcome> {
    token::logout(path, oidc, http).await
}

/// 既定のブラウザで開く。開けなくても URL は表示しているので、失敗は無視する。
fn open_browser(url: &str) {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(command)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    const API: &str = "https://api.example";

    fn oidc(issuer: &str) -> OidcConfig {
        OidcConfig {
            issuer: issuer.into(),
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn token(oidc: &OidcConfig, access: &str, refresh: &str) -> StoredTokens {
        StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: API.into(),
            access_token: access.into(),
            refresh_token: Some(refresh.into()),
            expires_at: token::now() + 600,
        }
    }

    #[tokio::test]
    async fn logout_does_not_delete_a_different_login_created_while_revoke_is_pending() {
        const MODE: &str = "ZOO2_TUI_LOGOUT_RACE_WORKER";
        if std::env::var(MODE).as_deref() == Ok("login") {
            let path = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_RACE_PATH").unwrap());
            let issuer = std::env::var("ZOO2_TUI_LOGOUT_RACE_ISSUER").unwrap();
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_RACE_READY").unwrap());
            let start = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_RACE_START").unwrap());
            std::fs::write(&ready, "ready").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let other_login = token(&oidc(&issuer), "new-account-access", "new-account-refresh");
            token::save(&path, &other_login).unwrap();
            return;
        }

        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=old-refresh"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = std::env::temp_dir()
            .join(format!("zoo2-tui-logout-race-{}", uuid::Uuid::new_v4()))
            .join("token.json");
        token::save(&path, &token(&config, "old-access", "old-refresh")).unwrap();

        let logout = tokio::spawn({
            let path = path.clone();
            let config = config.clone();
            let http = reqwest::Client::new();
            async move { revoke_and_delete_tokens(&path, &config, &http).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|request| request.url.path() == "/revoke")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("logout reached the revocation endpoint");

        let parent = path.parent().unwrap();
        let ready = parent.join("login-worker.ready");
        let start = parent.join("login-worker.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::logout_does_not_delete_a_different_login_created_while_revoke_is_pending")
            .arg("--nocapture")
            .env(MODE, "login")
            .env("ZOO2_TUI_LOGOUT_RACE_PATH", &path)
            .env("ZOO2_TUI_LOGOUT_RACE_ISSUER", &config.issuer)
            .env("ZOO2_TUI_LOGOUT_RACE_READY", &ready)
            .env("ZOO2_TUI_LOGOUT_RACE_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("independent login process is ready");
        std::fs::write(&start, "login").unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "login worker exited with {status}");
        assert_eq!(
            logout.await.unwrap().unwrap(),
            token::DeleteOutcome::Deleted
        );
        let retained = token::load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(retained.access_token, "new-account-access");
        assert_eq!(
            retained.refresh_token.as_deref(),
            Some("new-account-refresh")
        );
        token::delete(&path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn separate_process_logout_invalidates_an_unpersisted_refresh() {
        if std::env::var("ZOO2_TUI_LOGOUT_MEMORY_WORKER").is_ok() {
            let path = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_MEMORY_PATH").unwrap());
            let issuer = std::env::var("ZOO2_TUI_LOGOUT_MEMORY_ISSUER").unwrap();
            let outcome = revoke_and_delete_tokens(&path, &oidc(&issuer), &reqwest::Client::new())
                .await
                .unwrap();
            assert_eq!(outcome, token::DeleteOutcome::Deleted);
            return;
        }

        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/api/auth/oauth2/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "memory-only-access",
                "refresh_token": "memory-only-refresh",
                "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=old-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = std::env::temp_dir()
            .join(format!("zoo2-tui-memory-logout-{}", uuid::Uuid::new_v4()))
            .join("token.json");
        let mut initial = token(&config, "old-access", "old-refresh");
        initial.expires_at = token::now() + 30;
        token::save(&path, &initial).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let parent = path.parent().unwrap();
        let parent_mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o7777;
        std::fs::set_permissions(
            parent,
            std::fs::Permissions::from_mode(parent_mode & !0o222),
        )
        .unwrap();
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );
        assert!(manager.access_token().await.is_err());
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(parent_mode)).unwrap();

        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::separate_process_logout_invalidates_an_unpersisted_refresh")
            .arg("--nocapture")
            .env("ZOO2_TUI_LOGOUT_MEMORY_WORKER", "1")
            .env("ZOO2_TUI_LOGOUT_MEMORY_PATH", &path)
            .env("ZOO2_TUI_LOGOUT_MEMORY_ISSUER", &config.issuer)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "logout worker exited with {status}");
        let error = manager.access_token().await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<zoo2_tui::http::HttpError>(),
            Some(zoo2_tui::http::HttpError::Unauthorized)
        ));
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().any(|request| {
            request.method == "POST" && request.url.path() == "/api/auth/oauth2/token"
        }));
        assert!(
            requests
                .iter()
                .any(|request| { request.method == "POST" && request.url.path() == "/revoke" })
        );
        let revoked_tokens = requests
            .iter()
            .filter(|request| request.method == "POST" && request.url.path() == "/revoke")
            .map(|request| String::from_utf8_lossy(&request.body).into_owned())
            .collect::<Vec<_>>();
        assert!(
            revoked_tokens
                .iter()
                .any(|body| body.contains("token=old-refresh"))
        );
        assert!(
            revoked_tokens
                .iter()
                .all(|body| !body.contains("memory-only-refresh")),
            "another process cannot revoke a refresh token that was never persisted"
        );
        assert_eq!(token::load(&path, &config, Some(API)).unwrap(), None);
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[tokio::test]
    async fn logout_waits_for_refresh_then_revokes_its_latest_token() {
        const MODE: &str = "ZOO2_TUI_REFRESH_RACE_WORKER";
        if std::env::var(MODE).as_deref() == Ok("refresh") {
            let path = PathBuf::from(std::env::var_os("ZOO2_TUI_REFRESH_RACE_PATH").unwrap());
            let issuer = std::env::var("ZOO2_TUI_REFRESH_RACE_ISSUER").unwrap();
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_REFRESH_RACE_READY").unwrap());
            let start = PathBuf::from(std::env::var_os("ZOO2_TUI_REFRESH_RACE_START").unwrap());
            let config = oidc(&issuer);
            let tokens = token::load(&path, &config, Some(API)).unwrap().unwrap();
            std::fs::write(&ready, "ready").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let manager = TokenManager::new(reqwest::Client::new(), config, path, tokens);
            let result = manager.access_token().await;
            if std::env::var("ZOO2_TUI_REFRESH_RACE_EXPECT_LOGOUT").is_ok() {
                let error = result.unwrap_err();
                assert!(matches!(
                    error.downcast_ref::<zoo2_tui::http::HttpError>(),
                    Some(zoo2_tui::http::HttpError::Unauthorized)
                ));
            } else {
                assert_eq!(result.unwrap(), "rotated-access");
            }
            return;
        }

        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        let markers = std::env::temp_dir().join(format!(
            "zoo2-tui-refresh-race-markers-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&markers).unwrap();
        let token_request_started = markers.join("token-request.started");
        let logout_discovery_started = markers.join("logout-discovery.started");
        let discovery_requests_for_response = Arc::new(AtomicUsize::new(0));
        let issuer_for_response = issuer.clone();
        let server_uri = server.uri();
        let logout_discovery_marker = logout_discovery_started.clone();
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(move |_: &Request| {
                let discovery = json!({
                    "issuer": issuer_for_response.clone(),
                    "token_endpoint": format!("{server_uri}/token"),
                    "revocation_endpoint": format!("{server_uri}/revoke")
                });
                let response = ResponseTemplate::new(200).set_body_json(discovery);
                if discovery_requests_for_response.fetch_add(1, Ordering::SeqCst) == 0 {
                    response
                } else {
                    std::fs::write(&logout_discovery_marker, "waiting").unwrap();
                    response.set_delay(Duration::from_secs(1))
                }
            })
            .mount(&server)
            .await;
        let token_request_marker = token_request_started.clone();
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .respond_with(move |_: &Request| {
                std::fs::write(&token_request_marker, "refreshing").unwrap();
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(4))
                    .set_body_json(json!({
                        "access_token": "rotated-access",
                        "refresh_token": "rotated-refresh",
                        "expires_in": 300
                    }))
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=rotated-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = std::env::temp_dir()
            .join(format!(
                "zoo2-tui-logout-refresh-race-{}",
                uuid::Uuid::new_v4()
            ))
            .join("token.json");
        let mut initial = token(&config, "old-access", "old-refresh");
        initial.expires_at = token::now() + 30;
        token::save(&path, &initial).unwrap();
        let parent = path.parent().unwrap();
        let ready = parent.join("refresh-worker.ready");
        let start = parent.join("refresh-worker.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::logout_waits_for_refresh_then_revokes_its_latest_token")
            .arg("--nocapture")
            .env(MODE, "refresh")
            .env("ZOO2_TUI_REFRESH_RACE_PATH", &path)
            .env("ZOO2_TUI_REFRESH_RACE_ISSUER", &config.issuer)
            .env("ZOO2_TUI_REFRESH_RACE_READY", &ready)
            .env("ZOO2_TUI_REFRESH_RACE_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("refresh process loaded the same session snapshot");
        std::fs::write(&start, "refresh").unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !token_request_started.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("refresh process reached the delayed token endpoint");

        let logout = tokio::spawn({
            let path = path.clone();
            let config = config.clone();
            let http = reqwest::Client::new();
            async move { revoke_and_delete_tokens(&path, &config, &http).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !logout_discovery_started.exists(),
            "logout must wait for the refresh lock before choosing a token to revoke"
        );
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "refresh worker exited with {status}");
        assert_eq!(
            logout.await.unwrap().unwrap(),
            token::DeleteOutcome::Deleted
        );
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().any(|request| {
            request.method == "POST"
                && request.url.path() == "/revoke"
                && String::from_utf8_lossy(&request.body).contains("token=rotated-refresh")
        }));
        assert_eq!(token::load(&path, &config, Some(API)).unwrap(), None);
        std::fs::remove_dir_all(markers).unwrap();
    }

    #[tokio::test]
    async fn logout_times_out_slow_discovery_and_still_deletes_local_snapshot() {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(4))
                    .set_body_json(json!({
                        "issuer": issuer,
                        "token_endpoint": format!("{}/token", server.uri()),
                        "revocation_endpoint": format!("{}/revoke", server.uri())
                    })),
            )
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = std::env::temp_dir()
            .join(format!("zoo2-tui-slow-discovery-{}", uuid::Uuid::new_v4()))
            .join("token.json");
        token::save(&path, &token(&config, "access", "refresh")).unwrap();
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            revoke_and_delete_tokens(&path, &config, &reqwest::Client::new()),
        )
        .await;
        let removed = token::load(&path, &config, Some(API)).unwrap().is_none();
        if !removed {
            token::delete(&path).unwrap();
        }
        assert!(result.is_ok(), "logout exceeded the overall timeout");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            removed,
            "a revocation discovery failure still removes local tokens"
        );
    }

    #[tokio::test]
    async fn refresh_started_after_logout_snapshot_cannot_leave_that_session_logged_in() {
        const MODE: &str = "ZOO2_TUI_REFRESH_RACE_WORKER";
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        let markers = std::env::temp_dir().join(format!(
            "zoo2-tui-snapshot-refresh-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&markers).unwrap();
        let discovery_started = markers.join("logout-discovery.started");
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_response = calls.clone();
        let marker = discovery_started.clone();
        let issuer_for_response = issuer.clone();
        let server_uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(move |_: &Request| {
                let response = ResponseTemplate::new(200).set_body_json(json!({
                    "issuer": issuer_for_response.clone(),
                    "token_endpoint": format!("{server_uri}/token"),
                    "revocation_endpoint": format!("{server_uri}/revoke")
                }));
                if calls_for_response.fetch_add(1, Ordering::SeqCst) == 0 {
                    std::fs::write(&marker, "snapshot-taken").unwrap();
                    response.set_delay(Duration::from_secs(1))
                } else {
                    response
                }
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "rotated-access",
                "refresh_token": "rotated-refresh",
                "expires_in": 300
            })))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=old-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = std::env::temp_dir()
            .join(format!(
                "zoo2-tui-snapshot-refresh-{}",
                uuid::Uuid::new_v4()
            ))
            .join("token.json");
        let mut initial = token(&config, "old-access", "old-refresh");
        initial.expires_at = token::now() + 30;
        token::save(&path, &initial).unwrap();
        let parent = path.parent().unwrap();
        let ready = parent.join("refresh-worker.ready");
        let start = parent.join("refresh-worker.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::logout_waits_for_refresh_then_revokes_its_latest_token")
            .arg("--nocapture")
            .env(MODE, "refresh")
            .env("ZOO2_TUI_REFRESH_RACE_EXPECT_LOGOUT", "1")
            .env("ZOO2_TUI_REFRESH_RACE_PATH", &path)
            .env("ZOO2_TUI_REFRESH_RACE_ISSUER", &config.issuer)
            .env("ZOO2_TUI_REFRESH_RACE_READY", &ready)
            .env("ZOO2_TUI_REFRESH_RACE_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("refresh process loaded before logout took its snapshot");

        let logout = tokio::spawn({
            let path = path.clone();
            let config = config.clone();
            let http = reqwest::Client::new();
            async move { revoke_and_delete_tokens(&path, &config, &http).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !discovery_started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("logout took its snapshot and entered delayed discovery");
        std::fs::write(&start, "refresh").unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "refresh worker exited with {status}");
        assert_eq!(
            logout.await.unwrap().unwrap(),
            token::DeleteOutcome::Deleted
        );
        assert_eq!(token::load(&path, &config, Some(API)).unwrap(), None);
        std::fs::remove_dir_all(markers).unwrap();
    }
}
