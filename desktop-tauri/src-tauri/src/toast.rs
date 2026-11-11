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
