use anyhow::{Result, bail};
use zoo2_tui::{
    config::{AuthConfig, Config},
    http::Credentials,
    ui,
};

const USAGE: &str = "\
使い方: zoo2-tui [コマンド]

コマンド:
  run      チャット画面を開く (省略時の既定)
  help     このヘルプを表示する

設定は ~/.config/zoo2-tui/config.toml と環境変数 ZOO_* から読み込みます。";

#[tokio::main]
async fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());
    match command.as_str() {
        "run" => {
            let config = Config::load()?;
            let credentials = match &config.auth {
                AuthConfig::Dev { user_id } => Credentials::Dev(user_id.clone()),
                AuthConfig::Oidc(_) => {
                    bail!(
                        "OIDC ログインはまだ使えません。ZOO_DEV_USER を設定して dev モードで起動してください"
                    )
                }
            };
            ui::run(reqwest::Client::new(), config.api_base_url, credentials).await
        }
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
