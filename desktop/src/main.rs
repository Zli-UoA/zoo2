use anyhow::Result;
use zoo2_client_core::config::Config;
use zoo2_desktop::login::{APP_NAME, cli_login, cli_logout};
use zoo2_desktop::tokens::{ThemeMode, set_initial_mode};

const USAGE: &str = "\
使い方: zoo2-desktop [コマンド]

コマンド:
  run      デスクトップ画面を開く (省略時の既定)
  gallery  デザイン部品の確認画面を開く。`gallery components [ページ]` で UI 部品、`gallery shell [loading|welcome]` でサーバーなしのワークスペースの画面になる
  login    ブラウザでコードを承認してログインする
  logout   保存したトークンを消す
  help     このヘルプを表示する

オプション:
  --theme light|dark   gallery の配色を選ぶ (gallery だけに効く。環境変数 ZOO_DESKTOP_THEME でも指定できる。既定は light)

設定は ~/.config/zoo2-desktop/config.toml (OS ごとの設定ディレクトリ) と環境変数 ZOO_* から読み込みます。";

/// `--theme X` / `--theme=X` を引数から取り除き、指定された値をそのまま返す。
/// 値の検証はしない。どのコマンドでも取り除くが、使うのは gallery だけ。
/// 値が無いとき (`--theme` が末尾) は空文字を返し、gallery 側で不正な値として扱う。
fn strip_theme_arg(args: &mut Vec<String>) -> Option<String> {
    let mut value = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--theme" {
            value = Some(if i + 1 < args.len() {
                args.remove(i + 1)
            } else {
                String::new()
            });
            args.remove(i);
        } else if let Some(v) = args[i].strip_prefix("--theme=") {
            value = Some(v.to_string());
            args.remove(i);
        } else {
            i += 1;
        }
    }
    value
}

/// gallery の配色を決める。`--theme` を優先し、無ければ環境変数の値を見る。
/// 値があって light / dark でなければ、エラーメッセージを返す。
fn gallery_theme(
    explicit: Option<String>,
    env: Option<String>,
) -> Result<Option<ThemeMode>, String> {
    match explicit.or(env) {
        None => Ok(None),
        Some(value) => ThemeMode::parse(&value)
            .map(Some)
            .ok_or_else(|| format!("--theme には light か dark を指定してください: {value}")),
    }
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let theme_arg = strip_theme_arg(&mut args);
    let command = args.first().cloned().unwrap_or_else(|| "run".to_string());
    match command.as_str() {
        "run" => {
            // 2 つ目の起動は、既存のウィンドウを前に出して終わる。
            use zoo2_desktop::resident::single_instance::{Acquired, acquire};
            match acquire() {
                Acquired::Primary => {}
                Acquired::Secondary => return Ok(()),
                Acquired::Busy => {
                    anyhow::bail!(
                        "既に起動しているインスタンスが応答しないため、起動を中止しました"
                    )
                }
            }
            let config = Config::load_for(APP_NAME)?;
            zoo2_desktop::run(config);
            Ok(())
        }
        "gallery" => {
            match gallery_theme(theme_arg, std::env::var("ZOO_DESKTOP_THEME").ok()) {
                Ok(Some(mode)) => set_initial_mode(mode),
                Ok(None) => {}
                Err(message) => {
                    eprintln!("{message}\n\n{USAGE}");
                    std::process::exit(2);
                }
            }
            zoo2_desktop::gallery::run(&args[1..]);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn strip_removes_theme_for_any_command_without_validating() {
        let mut args = v(&["login", "--theme", "system"]);
        assert_eq!(strip_theme_arg(&mut args), Some("system".to_string()));
        assert_eq!(args, v(&["login"]));

        let mut args = v(&["--theme=", "help"]);
        assert_eq!(strip_theme_arg(&mut args), Some(String::new()));
        assert_eq!(args, v(&["help"]));

        let mut args = v(&["run", "--theme"]);
        assert_eq!(strip_theme_arg(&mut args), Some(String::new()));
        assert_eq!(args, v(&["run"]));

        let mut args = v(&["run"]);
        assert_eq!(strip_theme_arg(&mut args), None);
        assert_eq!(args, v(&["run"]));
    }

    #[test]
    fn gallery_theme_reads_explicit_then_env() {
        assert_eq!(gallery_theme(None, None), Ok(None));
        assert_eq!(
            gallery_theme(None, Some("dark".into())),
            Ok(Some(ThemeMode::Dark))
        );
        assert_eq!(
            gallery_theme(Some("light".into()), Some("dark".into())),
            Ok(Some(ThemeMode::Light))
        );
    }

    #[test]
    fn gallery_theme_rejects_bad_values() {
        assert!(gallery_theme(None, Some("system".into())).is_err());
        assert!(gallery_theme(None, Some(String::new())).is_err());
        assert!(gallery_theme(Some("blue".into()), Some("dark".into())).is_err());
    }

    #[test]
    fn bad_env_does_not_affect_other_commands() {
        // gallery 以外は gallery_theme を呼ばないので、環境変数が不正でも引数の取り除きだけで済む。
        for command in ["run", "login", "logout", "help"] {
            let mut args = v(&[command]);
            assert_eq!(strip_theme_arg(&mut args), None);
            assert_eq!(args, v(&[command]));
        }
    }
}
