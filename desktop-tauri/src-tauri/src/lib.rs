//! zoo2 のデスクトップ版 (Tauri v2)。設計は `docs/desktop-tauri.md` を参照。
//!
//! - `core`: ログイン状態、Shape の購読、通知の判定を持つ。
//! - `commands`: WebView から呼ぶコマンド。
//! - `toast`: OS の通知の表示。
//! - `window`: ウィンドウ、トレイ、単一インスタンス。

mod commands;
#[cfg(test)]
mod commands_list;
mod core;
mod toast;
mod window;

use std::sync::Arc;

use tauri::Manager;
use zoo2_client_core::config::Config;
use zoo2_client_types::ipc::UiMode;

/// 設定ディレクトリとトークンの置き場所の名前。GPUI 版 (`zoo2-desktop`) や TUI とは分ける。
pub const APP_NAME: &str = "zoo2-desktop-tauri";

/// `ZOO_DESKTOP_UI=leptos|remote`。既定は検証中なので `leptos`。
fn ui_mode() -> UiMode {
    match std::env::var("ZOO_DESKTOP_UI").as_deref() {
        Ok("remote") => UiMode::Remote,
        _ => UiMode::Leptos,
    }
}

pub fn run() {
    let ui = ui_mode();
    tauri::Builder::default()
        // 単一インスタンスは最初に登録する (2 つ目のプロセスを、他の初期化より前に止めるため)。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::zoo_session,
            commands::zoo_start_login,
            commands::zoo_cancel_login,
            commands::zoo_logout,
            commands::zoo_set_focus,
            commands::zoo_open_channel,
            commands::zoo_resync,
            commands::zoo_send_message,
            commands::zoo_access_token,
            commands::zoo_notifications_observed,
        ])
        .setup(move |app| {
            let config = Config::load_for(APP_NAME);
            let core = Arc::new(core::Core::new(app.handle().clone(), ui, config));
            app.manage(core.clone());
            window::add_remote_capability(app, &core)?;
            window::create_main(app, &core)?;
            window::create_tray(app)?;
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
    use zoo2_client_types::ipc::command;

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
            command::SEND_MESSAGE,
            command::ACCESS_TOKEN,
            command::NOTIFICATIONS_OBSERVED,
        ];
        assert_eq!(super::commands_list::COMMANDS, ipc);
    }
}
