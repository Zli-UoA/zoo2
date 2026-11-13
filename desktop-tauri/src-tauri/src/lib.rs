//! zoo2 のデスクトップ版 (Tauri v2)。設計は `docs/desktop-tauri.md` を参照。
//!
//! - `core`: ログイン状態、Shape の購読、通知の判定を持つ。
//! - `commands`: WebView から呼ぶコマンド。
//! - `toast`: OS の通知の表示。
//! - `window`: ウィンドウ、トレイ、単一インスタンス。
//! - `update`: アプリの自動更新。

mod commands;
#[cfg(test)]
mod commands_list;
mod core;
mod toast;
mod update;
mod window;

use std::sync::Arc;

use tauri::Manager;
use zoo2_client_core::config::{AuthConfig, Config, desktop_ui_for};
use zoo2_client_types::ipc::UiMode;

/// 設定ディレクトリとトークンの置き場所の名前。GPUI 版 (`zoo2-desktop`) や TUI とは分ける。
pub const APP_NAME: &str = "zoo2-desktop-tauri";

/// 画面の種類。`desktop_ui` (`ZOO_DESKTOP_UI`) で `remote` か `leptos` を選ぶ。
/// 指定がなければ、zli.chat の Web 版の画面 (`remote`) にする。ただし dev 認証 (`ZOO_DEV_USER`) で
/// ローカルの backend に向けているときは、backend が Web の画面を配信しないので、同梱の画面にする。
/// 知らない値はエラーにする (打ち間違いに気づけるように)。
fn ui_mode(desktop_ui: Option<&str>, dev_auth: bool) -> anyhow::Result<UiMode> {
    match desktop_ui {
        Some("remote") => Ok(UiMode::Remote),
        Some("leptos") => Ok(UiMode::Leptos),
        Some(other) => anyhow::bail!("desktop_ui は remote か leptos にしてください: {other}"),
        None if dev_auth => Ok(UiMode::Leptos),
        None => Ok(UiMode::Remote),
    }
}

pub fn run() {
    let mut config = Config::load_for(APP_NAME);
    let dev_auth = matches!(&config, Ok(config) if matches!(config.auth, AuthConfig::Dev { .. }));
    // desktop_ui が誤っているときは、設定のエラーとして同梱の画面に出す (「設定を読めません」)。
    let ui = match desktop_ui_for(APP_NAME).and_then(|ui| ui_mode(ui.as_deref(), dev_auth)) {
        Ok(ui) => ui,
        Err(err) => {
            if config.is_ok() {
                config = Err(err);
            }
            UiMode::Leptos
        }
    };
    tauri::Builder::default()
        // 単一インスタンスは最初に登録する (2 つ目のプロセスを、他の初期化より前に止めるため)。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::zoo_session,
            commands::zoo_start_login,
            commands::zoo_cancel_login,
            commands::zoo_logout,
            commands::zoo_set_focus,
            commands::zoo_open_channel,
            commands::zoo_resync,
            commands::zoo_account_profiles,
            commands::zoo_send_message,
            commands::zoo_access_token,
            commands::zoo_notifications_observed,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            toast::request_permission_macos();
            // 設定を読めないときは確かめる (直した版を届けられるように)。
            let check_updates = !matches!(&config, Ok(config) if !config.check_updates);
            let core = Arc::new(core::Core::new(app.handle().clone(), ui, config));
            app.manage(core.clone());
            window::add_remote_capability(app, &core)?;
            window::create_main(app, &core)?;
            window::create_tray(app)?;
            #[cfg(target_os = "macos")]
            window::create_app_menu(app)?;
            update::spawn(app.handle(), check_updates);
            core.start();
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .build(tauri::generate_context!())
        .expect("Tauri の初期化に失敗しました")
        .run(|_app, _event| {});
}

#[cfg(test)]
mod tests {
    use zoo2_client_types::ipc::{UiMode, command};

    #[test]
    fn ui_mode_defaults_to_remote_except_dev_auth() {
        use super::ui_mode;
        assert_eq!(ui_mode(None, false).unwrap(), UiMode::Remote);
        assert_eq!(ui_mode(None, true).unwrap(), UiMode::Leptos);
        assert_eq!(ui_mode(Some("leptos"), false).unwrap(), UiMode::Leptos);
        assert_eq!(ui_mode(Some("remote"), true).unwrap(), UiMode::Remote);
        assert!(ui_mode(Some("leptso"), false).is_err());
        assert!(ui_mode(Some("web"), true).is_err());
    }

    #[test]
    fn declared_commands_match_ipc_names() {
        let ipc = [
            command::SESSION,
            command::START_LOGIN,
            command::CANCEL_LOGIN,
            command::LOGOUT,
            command::SET_FOCUS,
            command::OPEN_CHANNEL,
            command::RESYNC,
            command::ACCOUNT_PROFILES,
            command::SEND_MESSAGE,
            command::ACCESS_TOKEN,
            command::NOTIFICATIONS_OBSERVED,
        ];
        assert_eq!(super::commands_list::COMMANDS, ipc);
    }
}
