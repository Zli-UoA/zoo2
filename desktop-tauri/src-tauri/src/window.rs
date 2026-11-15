//! メインウィンドウ、トレイ、ナビゲーションの制限。

use std::sync::Arc;

use tauri::{
    App, AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
    ipc::CapabilityBuilder,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    webview::{PermissionKind, PermissionResponse},
};
use tauri_plugin_opener::OpenerExt;
use zoo2_client_types::ipc::{OpenTarget, SessionState, UiMode, command, event};

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
    if let Some(url) = remote_url(core) {
        origins.push(url.origin().ascii_serialization());
    }
    origins.dedup();
    origins
}

/// (b) で読み込む URL。`api_base_url` の起点 (`https://zli.chat/` など)。
/// debug ビルドだけ、`ZOO_DESKTOP_REMOTE_URL` で差し替えられる (Vite の dev サーバーなど)。
pub fn remote_url(core: &Core) -> Option<Url> {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("ZOO_DESKTOP_REMOTE_URL")
    {
        return Url::parse(&url).ok();
    }
    let base = Url::parse(core.api_base_url()?).ok()?;
    base.join("/").ok()
}

/// 同梱の画面の URL。release では tauri://localhost、開発中は trunk の dev サーバー。
fn bundled_url() -> Url {
    let url = if cfg!(debug_assertions) {
        "http://localhost:1420/"
    } else {
        "tauri://localhost/"
    };
    Url::parse(url).expect("固定の URL")
}

/// (b) のリモートの画面に許すもの。トークンの受け渡しと通知の行の受け取りはここだけ。
const REMOTE_COMMANDS: &[&str] = &[
    command::SESSION,
    command::START_LOGIN,
    command::CANCEL_LOGIN,
    command::LOGOUT,
    command::SET_FOCUS,
    command::NOTIFICATION_SETTINGS,
    command::SET_NOTIFICATIONS,
    command::ACCESS_TOKEN,
    command::NOTIFICATIONS_OBSERVED,
    command::TAKE_CALL_ACCEPT,
];

/// (b) のときだけ、リモートのオリジンに IPC を許す capability を足す。
/// オリジンは設定 (`api_base_url`) で決まるので、JSON ではなくここで作る。
pub fn add_remote_capability(app: &App, core: &Core) -> tauri::Result<()> {
    if core.ui() != UiMode::Remote {
        return Ok(());
    }
    // `CapabilityBuilder::new` は `local: true` なので、外して同梱の画面には効かせない。
    let mut capability = CapabilityBuilder::new("main-remote")
        .local(false)
        .window(MAIN)
        .permission("core:event:allow-listen")
        .permission("core:event:allow-unlisten");
    for origin in remote_origins(core) {
        capability = capability.remote(format!("{origin}/*"));
    }
    for name in REMOTE_COMMANDS {
        capability = capability.permission(format!("allow-{}", name.replace('_', "-")));
    }
    app.add_capability(capability)
}

/// (b) では、ログインしたらリモートの画面へ、ログアウトしたら同梱のログイン画面へ移す。
pub fn follow_session(app: &AppHandle, core: &Core, state: &SessionState) {
    if core.ui() != UiMode::Remote {
        return;
    }
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let current = window.url().ok();
    let on_remote = current
        .as_ref()
        .is_some_and(|url| classify(url, core) == Origin::Remote);
    match state {
        SessionState::Ready { .. } if !on_remote => {
            if let Some(url) = remote_url(core) {
                let _ = window.navigate(url);
            }
        }
        SessionState::LoggedOut | SessionState::Failed { .. } if on_remote => {
            let _ = window.navigate(bundled_url());
        }
        _ => {}
    }
}

pub fn create_main(app: &App, core: &Arc<Core>) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let nav_core = core.clone();
    let perm_core = core.clone();
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
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
        // 通話 (getUserMedia) のマイクとカメラは、(b) の Web の画面にだけ許す。
        // macOS では、このあと OS がマイクの使用を尋ねる (Info.plist の説明文を出す)。
        .on_permission_request(move |webview, kind| {
            let remote = webview
                .url()
                .is_ok_and(|url| classify(&url, &perm_core) == Origin::Remote);
            match kind {
                PermissionKind::Microphone | PermissionKind::Camera if remote => {
                    PermissionResponse::Allow
                }
                _ => PermissionResponse::Default,
            }
        })
        .build()?;
    #[cfg(target_os = "linux")]
    enable_webrtc(&window);
    Ok(())
}

/// WebKitGTK は WebRTC と getUserMedia が既定で無効なので、通話のために有効にする。
#[cfg(target_os = "linux")]
fn enable_webrtc(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        use webkit2gtk::{SettingsExt, WebViewExt};
        if let Some(settings) = webview.inner().settings() {
            settings.set_enable_webrtc(true);
            settings.set_enable_media_stream(true);
        }
    });
}

/// macOS のメニューバー。Tauri の既定のメニュー (アプリ、編集、表示、ウィンドウなど) の、
/// アプリのメニューの「zoo2 について」の下に「アップデートを確認…」を足す。
#[cfg(target_os = "macos")]
pub fn create_app_menu(app: &App) -> tauri::Result<()> {
    use tauri::menu::{MenuItemKind, PredefinedMenuItem};

    let menu = Menu::default(app.handle())?;
    let item = MenuItem::with_id(
        app,
        crate::update::CHECK_MENU_ID,
        "アップデートを確認…",
        true,
        None::<&str>,
    )?;
    // 既定のアプリのメニューは「について、区切り、サービス、…」。区切りの後ろに、項目と区切りを入れる。
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        app_menu.insert(&item, 2)?;
        app_menu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id.as_ref() == crate::update::CHECK_MENU_ID {
            crate::update::check_now(app);
        }
    });
    app.manage(crate::update::CheckMenuItem(item));
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
            crate::update::MENU_ID => crate::update::install_and_restart(app),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(crate::update::TrayMenu(menu));
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
