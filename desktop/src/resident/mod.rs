//! 常駐と通知。Tauri 版 (`desktop-tauri`) と同じ挙動を GPUI 版に入れる。
//!
//! - `notify`: OS 通知 (notify-rust)。判定は `zoo2_client_core::notify::notifications_to_show`。
//! - `tray`: トレイアイコン (Linux は ksni、macOS は tray-icon)。
//! - `close`: 閉じる操作の扱い。X11 と macOS は最小化して常駐し、Wayland は終了する。
//! - `single_instance`: 2 つ目の起動を、既存のウィンドウを前に出して終える。
//!
//! OS のスレッド (通知のクリック、トレイ、ソケット) から GPUI へは [`ResidentEvent`] を送る。

pub mod close;
pub mod notify;
pub mod single_instance;
pub mod tray;

use std::sync::OnceLock;

use futures::{StreamExt, channel::mpsc};
use gpui::{AnyWindowHandle, App, Entity};

use crate::app::AppState;

/// OS 側のスレッドから GPUI へ送る依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidentEvent {
    /// ウィンドウを前に出す。
    Show,
    /// ウィンドウを前に出し、そのチャンネル (と投稿) を開く。
    /// `user_id` は通知を出したときのログイン中のユーザー。ログアウトやアカウントの切り替えの後に
    /// 古い通知をクリックしても、別のユーザーの画面で開かないように、合わなければ捨てる。
    Open {
        channel_id: String,
        message_id: Option<String>,
        user_id: String,
    },
    /// アプリを終了する。
    Quit,
}

static SENDER: OnceLock<mpsc::UnboundedSender<ResidentEvent>> = OnceLock::new();

/// どのスレッドからでも呼べる。まだ `install` されていなければ捨てる。
pub fn send(event: ResidentEvent) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.unbounded_send(event);
    }
}

/// ウィンドウを前に出す。最小化からの復帰には `activate_window` が要る (X11)。
pub fn show_window(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, _| window.activate_window());
    cx.activate(true);
}

/// イベントの受け口、閉じる操作の扱い、トレイ、単一インスタンスのサーバーを起動する。
pub fn install(cx: &mut App, state: Entity<AppState>, window: AnyWindowHandle) {
    let (tx, mut rx) = mpsc::unbounded();
    if SENDER.set(tx).is_err() {
        return;
    }

    let _ = window.update(cx, |_, window, cx| {
        close::install(window, cx, state.clone());
        // ウィンドウの前面かどうかは render の中でも写しているが、最小化 (X11 では unmap) の後は
        // 描画が止まって古い値が残る。通知の判定 (`focused_channel`) が狂うので、別に追う。
        state.update(cx, |_, cx| {
            cx.observe_window_activation(window, |state, window, cx| {
                state.update_timeline_view_state(
                    window.is_window_active(),
                    state.timeline_at_bottom,
                );
                cx.notify();
            })
            .detach();
        });
    });
    tray::spawn();
    single_instance::spawn_server();

    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            let _ = cx.update(|cx| match event {
                ResidentEvent::Show => show_window(window, cx),
                ResidentEvent::Open {
                    channel_id,
                    message_id,
                    user_id,
                } => {
                    // 前のユーザーの古い通知では、ウィンドウも前に出さない。
                    let is_current_user = state
                        .read(cx)
                        .me
                        .as_ref()
                        .is_some_and(|me| me.id == user_id);
                    if is_current_user {
                        show_window(window, cx);
                        state.update(cx, |state, cx| {
                            state.open_message(channel_id, message_id);
                            cx.notify();
                        });
                    }
                }
                ResidentEvent::Quit => cx.quit(),
            });
        }
    })
    .detach();
}
