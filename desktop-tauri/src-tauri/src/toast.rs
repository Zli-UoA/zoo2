//! OS の通知の表示。
//!
//! `tauri-plugin-notification` はデスクトップでクリックを受け取れないため、`notify-rust` を
//! 直接使う。Linux (freedesktop) では action を付け、専用のスレッドで `wait_for_action` を
//! 待つ。macOS では表示するだけで、クリックには対応しない (docs/desktop-tauri.md)。

use tauri::AppHandle;
use zoo2_client_types::ipc::OpenTarget;

/// macOS の通知を、このアプリ (bundle identifier) から出すようにする。
/// 指定しないと notify-rust は Finder として出そうとし、今の macOS では表示されないことがある。
#[cfg(target_os = "macos")]
pub fn init_macos(identifier: &str) {
    let _ = notify_rust::set_application(identifier);
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
            if markup {
                notification.body(&escape_markup(&body));
            }
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

/// freedesktop の通知の本文 (簡単な XML のマークアップ) で、文字として表示させる。
#[cfg(all(unix, not(target_os = "macos")))]
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    #[test]
    fn escapes_markup() {
        assert_eq!(
            super::escape_markup("a < b && <img src=x>"),
            "a &lt; b &amp;&amp; &lt;img src=x&gt;"
        );
    }
}
