//! OS の通知の表示。
//!
//! `tauri-plugin-notification` はデスクトップでクリックを受け取れないため、`notify-rust` を
//! 直接使う。Linux (freedesktop) では action を付け、専用のスレッドで `wait_for_action` を
//! 待つ。macOS では UNUserNotificationCenter で非同期に出すだけで、クリックには対応しない
//! (docs/desktop-tauri.md)。

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

    // macOS: 同期の `show()` は、メインスレッドの run loop がちょうど待機中でないと
    // `MainThreadNotRunning` で失敗する (通知はメイン以外のスレッドから出すため)。
    // 非同期の `show_async()` は run loop を待たないので、別のスレッドでこれを待つ。
    #[cfg(target_os = "macos")]
    {
        let _ = (app, on_click);
        std::thread::spawn(move || {
            if let Err(error) = tauri::async_runtime::block_on(notification.show_async()) {
                eprintln!("通知を出せませんでした: {error}");
            }
        });
    }

    #[cfg(not(unix))]
    {
        let _ = (app, on_click);
        let _ = notification.show();
    }
}

// ---- 通話の着信 (docs/calls.md) ----

use std::sync::{Arc, Mutex, PoisonError};

/// 着信のトーストで押せる操作。`accept` と `decline` は Linux だけ（mac はクリックを受け取れない）。
pub struct CallActions {
    /// 「応答」。
    pub accept: Option<Box<dyn FnOnce() + Send>>,
    /// 「拒否」。
    pub decline: Option<Box<dyn FnOnce() + Send>>,
    /// 本文のクリック。ウィンドウを前に出すだけで、応答はしない。
    pub open: Box<dyn FnOnce() + Send>,
}

#[derive(Default)]
enum CallToastState {
    /// まだ出していない（別のスレッドで出している途中）。
    #[default]
    Pending,
    Shown(Arc<notify_rust::NotificationHandle>),
    Closed,
}

/// 出している着信のトースト。`close` で、こちらから閉じる。
#[derive(Clone, Default)]
pub struct CallToast {
    state: Arc<Mutex<CallToastState>>,
}

impl CallToast {
    /// すでに閉じる指示が出ているか。出ていれば、OS に通知を出さない。
    pub fn is_closed(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            CallToastState::Closed
        )
    }

    /// 閉じる。出す前なら、出した直後に閉じる。何度呼んでもよい。
    pub fn close(&self) {
        let previous = std::mem::replace(
            &mut *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            CallToastState::Closed,
        );
        if let CallToastState::Shown(handle) = previous {
            std::thread::spawn(move || close_handle(&handle));
        }
    }

    /// 出し終えたら、ハンドルを預かる。すでに閉じる指示が出ていたら、すぐ閉じて `None`。
    fn shown(
        &self,
        handle: notify_rust::NotificationHandle,
    ) -> Option<Arc<notify_rust::NotificationHandle>> {
        let handle = Arc::new(handle);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, CallToastState::Closed) {
            drop(state);
            close_handle(&handle);
            return None;
        }
        *state = CallToastState::Shown(handle.clone());
        Some(handle)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn close_handle(handle: &notify_rust::NotificationHandle) {
    tauri::async_runtime::block_on(handle.close_async());
}

#[cfg(target_os = "macos")]
fn close_handle(handle: &notify_rust::NotificationHandle) {
    handle.close();
}

#[cfg(not(unix))]
fn close_handle(_handle: &notify_rust::NotificationHandle) {}

/// 着信のトーストを出す。Linux は「応答」「拒否」のボタン付きで、閉じるまで出し続ける
/// （GNOME は、通常の通知を数秒で一覧にしまうため）。mac は出すだけで、クリックは受け取れない。
/// ボタンを持てない環境（macOS など）で、受け取った操作を使わずに捨てる。
/// フィールドを読むので、`-D warnings` の lint（未使用のフィールド）に掛からない。
#[cfg(any(target_os = "macos", not(unix)))]
fn drop_unused_actions(actions: CallActions) {
    let CallActions {
        accept,
        decline,
        open,
    } = actions;
    drop((accept, decline, open));
}

/// 先に作っておいた `CallToast` に預かる。招待が終わったら、その `close` で閉じる（出す前でもよい）。
pub fn show_call_invite(toast: &CallToast, title: &str, body: &str, actions: CallActions) {
    // 閉じる指示が先に届いていたら、OS に通知を出さない（一瞬でも、ポップアップや通知音が出ないように）。
    if toast.is_closed() {
        return;
    }
    let mut notification = notify_rust::Notification::new();
    notification.appname("zoo2").summary(title).body(body);

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use notify_rust::{Hint, NotificationResponse, Timeout, Urgency};

        notification
            .hint(Hint::Urgency(Urgency::Critical))
            .timeout(Timeout::Never);
        if actions.accept.is_some() {
            notification.action("accept", "応答");
        }
        if actions.decline.is_some() {
            notification.action("decline", "拒否");
        }
        notification.action("default", "開く");
        let shared = toast.clone();
        let body = body.to_string();
        std::thread::spawn(move || {
            let markup = notify_rust::get_capabilities()
                .is_ok_and(|caps| caps.iter().any(|cap| cap == "body-markup"));
            notification.body(&zoo2_client_types::notify::body_for_server(&body, markup));
            // 能力の問い合わせの間に閉じる指示が届いていたら、出さない。
            if shared.is_closed() {
                return;
            }
            let Ok(handle) = notification.show() else {
                return;
            };
            let Some(handle) = shared.shown(handle) else {
                return;
            };
            let CallActions {
                mut accept,
                mut decline,
                open,
            } = actions;
            let mut open = Some(open);
            tauri::async_runtime::block_on(handle.wait_for_action_async(move |response| {
                match response {
                    NotificationResponse::Action(action) if action == "accept" => {
                        if let Some(accept) = accept.take() {
                            accept();
                        }
                    }
                    NotificationResponse::Action(action) if action == "decline" => {
                        if let Some(decline) = decline.take() {
                            decline();
                        }
                    }
                    NotificationResponse::Default => {
                        if let Some(open) = open.take() {
                            open();
                        }
                    }
                    _ => {}
                }
            }));
        });
    }

    #[cfg(target_os = "macos")]
    {
        let shared = toast.clone();
        // ボタンを持てない環境では、操作を使わない（フィールドを読んで捨てる）。
        drop_unused_actions(actions);
        std::thread::spawn(move || {
            if shared.is_closed() {
                return;
            }
            match tauri::async_runtime::block_on(notification.show_async()) {
                Ok(handle) => {
                    let _ = shared.shown(handle);
                }
                Err(error) => eprintln!("通知を出せませんでした: {error}"),
            }
        });
    }

    #[cfg(not(unix))]
    {
        // ボタンを持てない環境では、操作を使わない（フィールドを読んで捨てる）。
        drop_unused_actions(actions);
        let _ = notification.show();
    }
}
