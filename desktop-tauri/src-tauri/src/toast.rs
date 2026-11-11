//! OS の通知の表示。
//!
//! `tauri-plugin-notification` はデスクトップでクリックを受け取れないため、`notify-rust` を
//! 直接使う。Linux (freedesktop) では action を付け、専用のスレッドで `wait_for_action` を
//! 待つ。macOS では表示するだけで、クリックには対応しない (docs/desktop-tauri.md)。

use tauri::AppHandle;
use zoo2_client_types::ipc::OpenTarget;

pub fn show(app: &AppHandle, title: &str, body: &str, target: OpenTarget) {
    let mut notification = notify_rust::Notification::new();
    notification.appname("zoo2").summary(title).body(body);

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // "default" はクリックしたときの action (freedesktop の仕様)。
        notification.action("default", "開く");
        let app = app.clone();
        std::thread::spawn(move || {
            let Ok(handle) = notification.show() else {
                return;
            };
            handle.wait_for_action(|action| {
                if action == "default" {
                    crate::window::open_target(&app, target);
                }
            });
        });
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = (app, target);
        let _ = notification.show();
    }
}
