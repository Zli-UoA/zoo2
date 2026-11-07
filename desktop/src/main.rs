use anyhow::Result;
use zoo2_client_core::config::Config;
use zoo2_desktop::login::{APP_NAME, cli_login, cli_logout};

const USAGE: &str = "\
使い方: zoo2-desktop [コマンド]

コマンド:
  run      デスクトップ画面を開く (省略時の既定)
  login    ブラウザでコードを承認してログインする
  logout   保存したトークンを消す
  help     このヘルプを表示する

設定は ~/.config/zoo2-desktop/config.toml (OS ごとの設定ディレクトリ) と環境変数 ZOO_* から読み込みます。";

fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());
    match command.as_str() {
        "run" => {
            let config = Config::load_for(APP_NAME)?;
            zoo2_desktop::run(config);
            Ok(())
        }
        "login" => {
            let config = Config::load_for(APP_NAME)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(cli_login(&config))
        }
        "logout" => {
            let config = Config::load_for(APP_NAME)?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(cli_logout(&config))
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
