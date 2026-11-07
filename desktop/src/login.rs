//! OIDC デバイスコード認証・トークン管理と開発モード (`X-Dev-User`) 接続。

use std::{path::PathBuf, process::Stdio, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc::UnboundedSender;
use zoo2_client_core::{
    api::{ApiClient, Me},
    auth::{
        self,
        device::{self, Poller},
        token::{self, StoredTokens, TokenManager},
    },
    config::{self, AuthConfig, Config, OidcConfig},
    http::{Credentials, HttpError},
};

pub const APP_NAME: &str = "zoo2-desktop";

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
    Connected { credentials: Credentials, me: Me },
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
    let credentials = match &config.auth {
        AuthConfig::Dev { user_id } => Credentials::Dev(user_id.clone()),
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
            match token::load(&path, oidc, Some(&config.api_base_url)) {
                Ok(Some(tokens)) => Credentials::Bearer(Arc::new(TokenManager::new(
                    http.clone(),
                    oidc.clone(),
                    path,
                    tokens,
                ))),
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
            let _ = tx.send(AuthEvent::Connected { credentials, me });
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
    if let Err(err) = token::save(&path, &tokens) {
        let _ = tx.send(AuthEvent::LoginRequired {
            error: Some(format!("トークンの保存に失敗しました: {err:#}")),
        });
        return;
    }

    let manager = TokenManager::new(http.clone(), oidc.clone(), path, tokens);
    let credentials = Credentials::Bearer(Arc::new(manager));
    let api = ApiClient::new(http, &config.api_base_url, credentials.clone());
    match api
        .me()
        .await
        .context("ログインはできましたが、zoo2 がトークンを受け付けませんでした")
    {
        Ok(me) => {
            let _ = tx.send(AuthEvent::Connected { credentials, me });
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

/// OS の既定ブラウザで URL を開く (macOS / Windows / Linux 対応)。
pub fn open_browser(url: &str) {
    let mut command = if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    } else if cfg!(target_os = "windows") {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "start", "", url]);
        cmd
    } else {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
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
}
