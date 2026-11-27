//! メインウィンドウ、トレイ、ナビゲーションの制限。

use std::sync::Arc;

use tauri::{
    App, AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
    ipc::CapabilityBuilder,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    webview::{NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse},
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
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

/// 同梱の画面のオリジンか。プラットフォームごとに完全一致で判定する。
///
/// macOS と Linux は `tauri://localhost`、Windows は `http(s)://tauri.localhost` (ポートなし)。
/// ほかのプラットフォームのオリジンや、ポート付きのループバックは同梱として扱わない。
/// 開発中は trunk の dev サーバー (tauri.conf.json の devUrl)。
fn is_bundled_url(url: &Url, windows: bool) -> bool {
    let exact_host = |host: &str| url.host_str() == Some(host) && url.port().is_none();
    let platform = if windows {
        matches!(url.scheme(), "http" | "https") && exact_host("tauri.localhost")
    } else {
        url.scheme() == "tauri" && exact_host("localhost")
    };
    platform
        || (cfg!(debug_assertions) && url.origin().ascii_serialization() == "http://localhost:1420")
}

pub fn classify(url: &Url, core: &Core) -> Origin {
    classify_url(url, core.ui(), &remote_origins(core))
}

fn classify_url(url: &Url, ui: UiMode, allowed_remote_origins: &[String]) -> Origin {
    let origin = url.origin().ascii_serialization();
    let bundled = is_bundled_url(url, cfg!(windows));
    if bundled {
        return Origin::Bundled;
    }
    if ui == UiMode::Remote && allowed_remote_origins.contains(&origin) {
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

/// 確かめずに既定のブラウザーで開くオリジン。Web 版 (`api_base_url`、`zli.chat` など) と、
/// ログインの発行者 (`account.zli.works` など)。
fn trusted_origins(core: &Core) -> Vec<String> {
    let mut origins = remote_origins(core);
    if let Some(issuer) = core
        .oidc_issuer()
        .and_then(|issuer| Url::parse(issuer).ok())
    {
        origins.push(issuer.origin().ascii_serialization());
    }
    origins
}

/// 確かめるダイアログに出す URL の長さの上限 (長いクエリで、ダイアログが画面からはみ出ないように)。
const DIALOG_URL_MAX_CHARS: usize = 300;

/// アプリの中で開かないリンク (メッセージ中のリンクなど) を、既定のブラウザーで開く。
/// Web 版とログインの発行者のほかのサイトは、開く前にダイアログで確かめる。
pub fn open_external(app: &AppHandle, core: &Core, url: &Url) {
    match url.scheme() {
        "mailto" => {
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
            return;
        }
        "http" | "https" => {}
        _ => return,
    }
    if trusted_origins(core).contains(&url.origin().ascii_serialization()) {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
        return;
    }
    let shown: String = url.as_str().chars().take(DIALOG_URL_MAX_CHARS).collect();
    let ellipsis = if shown.len() < url.as_str().len() {
        "…"
    } else {
        ""
    };
    let host = url.host_str().unwrap_or_default();
    let mut dialog = app
        .dialog()
        .message(format!(
            "zoo2 の外のサイト ({host}) を、既定のブラウザーで開きます。\n\n{shown}{ellipsis}"
        ))
        .title("外部のサイトを開きます")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "開く".into(),
            "キャンセル".into(),
        ));
    if let Some(window) = app.get_webview_window(MAIN) {
        dialog = dialog.parent(&window);
    }
    let app = app.clone();
    let url = url.clone();
    dialog.show(move |ok| {
        if ok {
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
        }
    });
}

/// 同梱の画面の URL。release では macOS・Linux が tauri://localhost、Windows が http://tauri.localhost。
/// 開発中は trunk の dev サーバー。
fn bundled_url() -> Url {
    let url = if cfg!(debug_assertions) {
        "http://localhost:1420/"
    } else if cfg!(target_os = "windows") {
        "http://tauri.localhost/"
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
    command::SET_BADGE,
    command::NOTIFICATION_SETTINGS,
    command::SET_NOTIFICATIONS,
    command::ACCESS_TOKEN,
    command::NOTIFICATIONS_OBSERVED,
    command::TAKE_CALL_ACCEPT,
    command::REAUTHENTICATE,
    command::DOWNLOAD_SETTINGS,
    command::SET_DOWNLOAD_SETTINGS,
    command::PICK_DOWNLOAD_DIRECTORY,
    command::DOWNLOAD_BEGIN,
    command::DOWNLOAD_WRITE,
    command::DOWNLOAD_FINISH,
    command::DOWNLOAD_CANCEL,
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
    // ログアウトや期限切れでは画面が破棄され、React から `zoo_download_cancel` を呼べない。
    // 書き込み中の `.part` をここで消す。
    if matches!(state, SessionState::LoggedOut | SessionState::Failed { .. }) {
        core.downloads().cancel_all();
    }
    // ログアウトや期限切れでは、画面が破棄されて React の後始末が走らないことがある。
    // バッジは OS 側に残るので、ここで消す。
    if matches!(state, SessionState::LoggedOut | SessionState::Failed { .. })
        && let Some(window) = app.get_webview_window(MAIN)
    {
        let _ = window.set_badge_count(None);
    }
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
    let popup_handle = app.handle().clone();
    let popup_core = core.clone();
    let perm_core = core.clone();
    let page_core = core.clone();
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    let window = {
        let builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
            .title(format!("zoo2 v{}", env!("CARGO_PKG_VERSION")))
            .inner_size(1100.0, 760.0)
            .min_inner_size(480.0, 360.0)
            .on_navigation(move |url| {
                if classify(url, &nav_core) != Origin::Other {
                    return true;
                }
                // 許したオリジン以外 (メッセージ中のリンクなど) は、既定のブラウザーで開く。
                open_external(&handle, &nav_core, url);
                false
            })
            // `target="_blank"` や `window.open` も、新しいウィンドウは作らず、既定のブラウザーで開く。
            .on_new_window(move |url, _features| {
                open_external(&popup_handle, &popup_core, &url);
                NewWindowResponse::Deny
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
            // 画面を読み込み直すと React の後始末が走らず、`.part` が残る。新しい画面を
            // 読み始めた時点で、書き込み中のものをすべて中止する。
            .on_page_load(move |_window, payload| {
                if payload.event() == PageLoadEvent::Started {
                    page_core.downloads().cancel_all();
                }
            });
        #[cfg(target_os = "windows")]
        let builder = if let Ok(dir) = app.path().app_local_data_dir() {
            // WebView2 の保存データもアンインストール対象 (%LOCALAPPDATA%\chat.zli.desktop) に揃える。
            builder.data_directory(dir)
        } else {
            builder
        };
        builder.build()?
    };
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

/// メニューバー (macOS) の「再読み込み」の ID。
#[cfg(target_os = "macos")]
const RELOAD_MENU_ID: &str = "reload";
/// トレイの「再読み込み」の ID。メニューのイベントは、アプリのメニューとトレイの両方の
/// ハンドラーに届くので、メニューバーとは別の ID にする (同じだと 2 回読み込み直す)。
const TRAY_RELOAD_MENU_ID: &str = "tray_reload";

/// 画面を読み込み直す。リモートの画面 (`zli.chat` など) が新しくなったときに、アプリを
/// 再起動せずに取り込むため。明示的な読み込み直しなので、Service Worker は保存済みの画面ではなく
/// サーバーの画面を返す。
pub fn reload_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.reload();
    }
}

/// macOS のメニューバー。Tauri の既定のメニュー (アプリ、編集、表示、ウィンドウなど) の、
/// アプリのメニューの「zoo2 について」の下に「アップデートを確認…」を足し、「表示」の先頭に
/// 「再読み込み」(⌘R) を足す。
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
    let items = menu.items()?;
    if let Some(MenuItemKind::Submenu(app_menu)) = items.first() {
        app_menu.insert(&item, 2)?;
        app_menu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    }
    let reload = MenuItem::with_id(app, RELOAD_MENU_ID, "再読み込み", true, Some("CmdOrCtrl+R"))?;
    // 既定のメニューの「表示」(View)。見つからなければ、アプリのメニューに入れる。
    let view = items.iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) if submenu.text().is_ok_and(|text| text == "View") => {
            Some(submenu.clone())
        }
        _ => None,
    });
    match (view, items.first()) {
        (Some(view), _) => {
            view.insert(&reload, 0)?;
            view.insert(&PredefinedMenuItem::separator(app)?, 1)?;
        }
        (None, Some(MenuItemKind::Submenu(app_menu))) => app_menu.insert(&reload, 3)?,
        _ => {}
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| match event.id.as_ref() {
        crate::update::CHECK_MENU_ID => crate::update::check_now(app),
        RELOAD_MENU_ID => reload_main(app),
        _ => {}
    });
    app.manage(crate::update::CheckMenuItem(item));
    Ok(())
}

pub fn create_tray(app: &App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "開く", true, None::<&str>)?;
    let reload = MenuItem::with_id(app, TRAY_RELOAD_MENU_ID, "再読み込み", true, None::<&str>)?;
    #[cfg(not(target_os = "macos"))]
    let check_update = MenuItem::with_id(
        app,
        crate::update::CHECK_MENU_ID,
        "アップデートを確認…",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    #[cfg(target_os = "macos")]
    let menu = Menu::with_items(app, &[&open, &reload, &quit])?;
    #[cfg(not(target_os = "macos"))]
    let menu = Menu::with_items(app, &[&open, &reload, &check_update, &quit])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("zoo2")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            TRAY_RELOAD_MENU_ID => {
                show_main(app);
                reload_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            crate::update::CHECK_MENU_ID => crate::update::check_now(app),
            "quit" => {
                crate::update::prepare_manual_exit(app);
                app.exit(0);
            }
            #[cfg(target_os = "macos")]
            crate::update::MENU_ID => crate::update::install_and_restart(app),
            _ => {}
        });
    #[cfg(target_os = "windows")]
    {
        tray = tray
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(tray.app_handle());
                }
            });
    }
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    #[cfg(not(target_os = "macos"))]
    app.manage(crate::update::CheckMenuItem(check_update));
    #[cfg(target_os = "macos")]
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

#[cfg(test)]
mod bundled_origin_tests {
    use super::*;

    fn bundled(url: &str, windows: bool) -> bool {
        is_bundled_url(&Url::parse(url).unwrap(), windows)
    }

    #[test]
    fn bundled_origin_is_matched_exactly_per_platform() {
        assert!(bundled("tauri://localhost/index.html", false));
        assert!(bundled("http://tauri.localhost/index.html", true));
        assert!(bundled("https://tauri.localhost/", true));
        // 別のプラットフォームのオリジン
        assert!(!bundled("http://tauri.localhost/", false));
        assert!(!bundled("tauri://localhost/", true));
        // ポート付きのループバックや、似たホスト名
        assert!(!bundled("http://tauri.localhost:8123/x", false));
        assert!(!bundled("http://tauri.localhost:8123/x", true));
        assert!(!bundled("tauri://localhost:1/", false));
        assert!(!bundled("http://evil.tauri.localhost/", true));
        assert!(!bundled("ftp://tauri.localhost/", true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_url_matches_platform_and_build_profile() {
        let url = bundled_url();
        if cfg!(debug_assertions) {
            assert_eq!(url.as_str(), "http://localhost:1420/");
        } else if cfg!(target_os = "windows") {
            assert_eq!(url.as_str(), "http://tauri.localhost/");
        } else {
            assert_eq!(url.as_str(), "tauri://localhost/");
        }
    }

    #[test]
    fn classify_url_recognizes_bundled_remote_and_other() {
        let remote = vec!["https://zli.chat".to_string()];
        // 同梱のオリジンはプラットフォームごとに完全一致で判定する。
        let bundled_urls: &[&str] = if cfg!(windows) {
            &[
                "http://tauri.localhost/index.html",
                "https://tauri.localhost/index.html",
            ]
        } else {
            &["tauri://localhost/index.html"]
        };
        for bundled in bundled_urls {
            let parsed = Url::parse(bundled).unwrap();
            assert_eq!(
                classify_url(&parsed, UiMode::Remote, &remote),
                Origin::Bundled
            );
        }
        assert_eq!(
            classify_url(
                &Url::parse("https://zli.chat/channels/1").unwrap(),
                UiMode::Remote,
                &remote
            ),
            Origin::Remote
        );
        assert_eq!(
            classify_url(
                &Url::parse("https://zli.chat/channels/1").unwrap(),
                UiMode::Leptos,
                &remote
            ),
            Origin::Other
        );
        assert_eq!(
            classify_url(
                &Url::parse("https://example.com").unwrap(),
                UiMode::Remote,
                &remote
            ),
            Origin::Other
        );
    }
}
