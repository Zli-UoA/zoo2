//! メインウィンドウ、トレイ、ナビゲーションの制限。

use std::sync::Arc;

use tauri::{
    App, AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_opener::OpenerExt;
use zoo2_client_types::ipc::{OpenTarget, UiMode, event};

use crate::core::Core;

pub const MAIN: &str = "main";

/// 開発中に (b) で読み込む Vite の dev サーバー。debug ビルドだけで許す。
#[cfg(debug_assertions)]
const DEV_REMOTE_ORIGINS: &[&str] = &["http://localhost:5173"];
#[cfg(not(debug_assertions))]
const DEV_REMOTE_ORIGINS: &[&str] = &[];

/// 呼び出し元や移動先の URL がどこか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 同梱の画面 (Leptos とログイン画面)。
    Bundled,
    /// (b) で読み込む Web (`api_base_url` のオリジン)。
    Remote,
    Other,
}

pub fn classify(url: &Url, core: &Core) -> Origin {
    let origin = url.origin().ascii_serialization();
    // 同梱の画面は、macOS と Linux では tauri://localhost、Windows では http(s)://tauri.localhost。
    // 開発中は trunk の dev サーバー (tauri.conf.json の devUrl)。
    let bundled = url.scheme() == "tauri"
        || url.host_str() == Some("tauri.localhost")
        || (cfg!(debug_assertions) && origin == "http://localhost:1420");
    if bundled {
        return Origin::Bundled;
    }
    if core.ui() == UiMode::Remote && remote_origins(core).contains(&origin) {
        return Origin::Remote;
    }
    Origin::Other
}

/// (b) で読み込んでよいオリジン。
pub fn remote_origins(core: &Core) -> Vec<String> {
    let mut origins: Vec<String> = DEV_REMOTE_ORIGINS.iter().map(|o| o.to_string()).collect();
    if let Some(origin) = core
        .api_base_url()
        .and_then(|base| Url::parse(base).ok())
        .map(|url| url.origin().ascii_serialization())
    {
        origins.push(origin);
    }
    origins
}

pub fn create_main(app: &App, core: &Arc<Core>) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let nav_core = core.clone();
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("zoo2")
        .inner_size(1100.0, 760.0)
        .min_inner_size(480.0, 360.0)
        .on_navigation(move |url| {
            if classify(url, &nav_core) != Origin::Other {
                return true;
            }
            // 許したオリジン以外 (メッセージ中のリンクなど) は、既定のブラウザーで開く。
            if matches!(url.scheme(), "http" | "https" | "mailto") {
                let _ = handle.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        })
        .build()?;
    Ok(())
}

pub fn create_tray(app: &App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "開く", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("zoo2")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 通知などから、あるチャンネルを開く。
pub fn open_target(app: &AppHandle, target: OpenTarget) {
    show_main(app);
    let _ = app.emit_to(MAIN, event::OPEN, target);
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    match event {
        // 閉じたら隠し、トレイに残す。終了はトレイのメニューから。
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = window.hide();
        }
        WindowEvent::Focused(focused) => {
            let core = window.state::<Arc<Core>>().inner().clone();
            let focused = *focused;
            tauri::async_runtime::spawn(async move {
                core.set_window_focused(focused).await;
            });
        }
        _ => {}
    }
}
