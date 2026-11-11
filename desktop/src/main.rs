use anyhow::Result;
use zoo2_client_core::config::Config;
use zoo2_desktop::login::{APP_NAME, cli_login, cli_logout};
use zoo2_desktop::tokens::{ThemeMode, set_initial_mode};

const USAGE: &str = "\
使い方: zoo2-desktop [コマンド]

コマンド:
  run      デスクトップ画面を開く (省略時の既定)
  gallery  デザイン部品 (色・フォント・アイコン) の確認画面を開く
  login    ブラウザでコードを承認してログインする
  logout   保存したトークンを消す
  help     このヘルプを表示する

オプション:
  --theme light|dark   gallery の配色を選ぶ (gallery だけに効く。環境変数 ZOO_DESKTOP_THEME でも指定できる。既定は light)

設定は ~/.config/zoo2-desktop/config.toml (OS ごとの設定ディレクトリ) と環境変数 ZOO_* から読み込みます。";

/// `--theme X` / `--theme=X` を引数から取り除いて返す。無ければ環境変数を見る。
fn take_theme(args: &mut Vec<String>) -> Option<ThemeMode> {
    let mut value = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--theme" {
            if i + 1 >= args.len() {
                eprintln!("--theme には light か dark を指定してください\n\n{USAGE}");
                std::process::exit(2);
            }
            value = Some(args.remove(i + 1));
            args.remove(i);
        } else if let Some(v) = args[i].strip_prefix("--theme=") {
            value = Some(v.to_string());
            args.remove(i);
        } else {
            i += 1;
        }
    }
    let value = value.or_else(|| std::env::var("ZOO_DESKTOP_THEME").ok())?;
    match ThemeMode::parse(&value) {
        Some(mode) => Some(mode),
        None => {
            eprintln!("--theme には light か dark を指定してください: {value}");
            std::process::exit(2);
        }
    }
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(mode) = take_theme(&mut args) {
        set_initial_mode(mode);
    }
    let command = args.first().cloned().unwrap_or_else(|| "run".to_string());
    match command.as_str() {
        "run" => {
            let config = Config::load_for(APP_NAME)?;
            zoo2_desktop::run(config);
            Ok(())
        }
        "gallery" => {
            zoo2_desktop::gallery::run();
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
