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
            let tokens =
                token::load(&path, oidc, Some(&config.api_base_url))?.ok_or_else(|| {
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
    let crypto_task = config::config_dir().map(|dir| {
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
    let result = ui::run(http, config.api_base_url, credentials, config.check_updates).await;
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
    if !clear_encryption(&config, oidc, &path, &http).await? {
        println!("ログアウトを取りやめました。");
        return Ok(());
    }
    // IdP 側でも refresh_token を失効させる。失敗しても手元のトークンは消す。
    if let Some(refresh_token) =
        token::load(&path, oidc, None)?.and_then(|tokens| tokens.refresh_token)
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

/// 暗号ストアを消し、端末を削除する（docs/e2ee.md の「ログアウトと端末の削除」）。
///
/// 鍵バックアップに送れていない鍵があれば、件数を示して確かめる。取りやめたら `false`。
async fn clear_encryption(
    config: &Config,
    oidc: &OidcConfig,
    token_path: &std::path::Path,
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
            "{pending} 件の鍵がバックアップされていません。ログアウトすると、この端末にだけある過去ログは読めなくなります。"
        );
        print!("ログアウトしますか？ [y/N] ");
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(false);
        }
    }
    // 端末の削除 API にはトークンが要る。トークンがなければ、次のログインで消し直す。
    let credentials = match token::load(token_path, oidc, Some(&config.api_base_url))? {
        Some(tokens) => Credentials::Bearer(Arc::new(TokenManager::new(
            http.clone(),
            oidc.clone(),
            token_path.to_path_buf(),
            tokens,
        ))),
        None => Credentials::Dev(String::new()),
    };
    let api = ApiClient::new(http.clone(), &config.api_base_url, credentials);
    for target in targets {
        e2ee::manager::destroy_for_logout(&api, &root, target).await?;
    }
    Ok(true)
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
