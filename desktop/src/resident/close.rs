//! 閉じる操作の扱い。
//!
//! gpui の Linux は最後のウィンドウが閉じるとイベントループが止まる。
//! - X11 と macOS: 閉じる操作を止めて最小化し、トレイから戻せるようにする。
//!   X11 で最小化から戻すには `Window::activate_window` が要る。
//! - Wayland: gpui には最小化からの復帰がなく、`activate` も拒否されやすい。戻す手段がないので、
//!   閉じたら終了する。

use gpui::{App, Window};

/// 閉じたときに常駐できる環境か。
pub fn can_stay_resident() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        gpui::guess_compositor() == "X11"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
    {
        false
    }
}

pub fn install(window: &mut Window, cx: &App) {
    if !can_stay_resident() {
        return;
    }
    window.on_window_should_close(cx, |window, _cx| {
        window.minimize_window();
        false
    });
}
