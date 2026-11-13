//! OS の通知の表示。
//!
//! `tauri-plugin-notification` はデスクトップでクリックを受け取れないため、`notify-rust` を
//! 直接使う。Linux (freedesktop) では action を付け、専用のスレッドで `wait_for_action` を
//! 待つ。macOS では表示するだけで、クリックには対応しない (docs/desktop-tauri.md)。

use tauri::AppHandle;
use zoo2_client_types::ipc::OpenTarget;

/// macOS で、起動時に通知の許可を求める (初回は OS が尋ねる。2 回目からは前の答えを返す)。
/// 通知は UNUserNotificationCenter で出す (notify-rust の `preview-macos-un`)。
#[cfg(target_os = "macos")]
pub fn request_permission_macos() {
    // 答えを待つ間、起動を止めないよう別のスレッドで待つ。
    std::thread::spawn(|| match notify_rust::request_auth_blocking() {
        Ok(true) => {}
        Ok(false) => eprintln!(
            "通知が許可されていません。「システム設定」の「通知」で zoo2 をオンにしてください"
        ),
        Err(error) => eprintln!("通知の許可を求められませんでした: {error}"),
    });
}

/// 投稿の通知。クリックしたら、そのチャンネルを開く。
pub fn show(app: &AppHandle, title: &str, body: &str, target: OpenTarget) {
    show_with(app, title, body, move |app| {
        crate::window::open_target(app, target);
    });
}

/// 通知を出し、クリックされたら `on_click` を呼ぶ (Linux だけ)。
pub fn show_with(
    app: &AppHandle,
    title: &str,
    body: &str,
    on_click: impl FnOnce(&AppHandle) + Send + 'static,
) {
    let mut notification = notify_rust::Notification::new();
    notification.appname("zoo2").summary(title).body(body);

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // "default" はクリックしたときの action (freedesktop の仕様)。
        notification.action("default", "開く");
        let app = app.clone();
        let body = body.to_string();
        std::thread::spawn(move || {
            // 本文をマークアップとして読む通知サーバーでは、投稿の `<` や `&` がタグとして
            // 扱われないようにエスケープする。
            let markup = notify_rust::get_capabilities()
                .is_ok_and(|caps| caps.iter().any(|cap| cap == "body-markup"));
            notification.body(&zoo2_client_types::notify::body_for_server(&body, markup));
            let Ok(handle) = notification.show() else {
                return;
            };
            let mut on_click = Some(on_click);
            handle.wait_for_action(|action| {
                if action == "default"
                    && let Some(on_click) = on_click.take()
                {
                    on_click(&app);
                }
            });
        });
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = (app, on_click);
        let _ = notification.show();
    }
}
