use std::{path::PathBuf, process::Stdio, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use zoo2_tui::{
    api::ApiClient,
    auth::{
        self,
        device::{self, Poller},
        token::{self, StoredTokens, TokenManager},
    },
    config::{self, AuthConfig, Config, OidcConfig},
    http::Credentials,
    ui,
};

const USAGE: &str = "\
使い方: zoo2-tui [コマンド]

コマンド:
  run      チャット画面を開く (省略時の既定)
  login    ブラウザでコードを承認してログインする
  logout   保存したトークンを消す
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
            bail!("OIDC が設定されていません。oidc_issuer と oidc_client_id を設定してください")
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
            let tokens = token::load(&path, oidc)?.ok_or_else(|| {
                anyhow!("ログインしていません。先に `zoo2-tui login` を実行してください")
            })?;
            Credentials::Bearer(Arc::new(TokenManager::new(
                http.clone(),
                oidc.clone(),
                path,
                tokens,
            )))
        }
    };
    ui::run(http, config.api_base_url, credentials).await
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
    let tokens = StoredTokens::new(oidc, response, token::now());
    let path = token_file()?;
    token::save(&path, &tokens)?;

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
    let oidc = oidc_config(&config)?;
    let path = token_file()?;
    let http = reqwest::Client::new();
    // IdP 側でも refresh_token を失効させる。失敗しても手元のトークンは消す。
    if let Some(refresh_token) = token::load(&path, oidc)?.and_then(|tokens| tokens.refresh_token)
        && let Ok(discovery) = auth::discover(&http, &oidc.issuer).await
        && let Some(endpoint) = discovery.revocation_endpoint
    {
        let form = [
            ("token", refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
            ("client_id", oidc.client_id.as_str()),
        ];
        let _ = http.post(endpoint).form(&form).send().await;
    }
    if token::delete(&path)? {
        println!("ログアウトしました。");
    } else {
        println!("ログインしていません。");
    }
    Ok(())
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
