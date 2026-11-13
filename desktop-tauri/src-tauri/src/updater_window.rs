//! 更新のウィンドウ。アイコン、状態の文言、進捗バーだけの小さなウィンドウで、利用者が
//! 更新を始めたとき (メニューの「アップデートを確認…」、トレイや通知の「更新して再起動」) に出す。
//! 画面は同梱の `ui/updater/` (trunk が `dist/updater/` に置く)。
//!
//! 画面には IPC を許さない。状態は Rust 側から `window.zooUpdater.set(...)` を `eval` で渡す。
//! 読み込みが終わる前に渡した状態は、読み込みが終わったとき (`on_page_load`) に渡し直す。

use std::{sync::Mutex, time::Duration};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, webview::PageLoadEvent};

const LABEL: &str = "updater";

/// 進捗バー。
#[derive(Clone, Copy)]
pub enum Bar {
    /// 進み具合が分からない (動き続けるバー)。
    Indeterminate,
    /// 0.0〜1.0。
    Ratio(f64),
    Hidden,
}

/// 画面に出す状態。
pub struct View<'a> {
    pub title: &'a str,
    pub detail: &'a str,
    pub bar: Bar,
    /// アイコンを動かす (作業中)。
    pub busy: bool,
    pub error: bool,
}

/// 最後に渡した状態 (読み込みが終わったときに渡し直す)。
#[derive(Default)]
struct Last(Mutex<Option<String>>);

/// ウィンドウを出す (出ていれば前に出す)。
pub fn open(app: &AppHandle) {
    if app.try_state::<Last>().is_none() {
        app.manage(Last::default());
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("updater/index.html".into()))
            .title("zoo2 の更新")
            .inner_size(300.0, 340.0)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .always_on_top(true)
            .center()
            .on_page_load(|window, payload| {
                if payload.event() == PageLoadEvent::Finished
                    && let Some(last) = window.app_handle().try_state::<Last>()
                    && let Some(json) = lock(&last.0).clone()
                {
                    let _ = window.eval(script(&json));
                }
            })
            .build();
    if let Err(err) = result {
        eprintln!("更新のウィンドウを開けませんでした: {err}");
    }
}

/// 状態を変える。ウィンドウが出ていなければ、覚えておくだけ。
pub fn set(app: &AppHandle, view: View<'_>) {
    let progress = match view.bar {
        Bar::Indeterminate => Value::Null,
        Bar::Ratio(ratio) => json!(ratio.clamp(0.0, 1.0)),
        Bar::Hidden => json!("hidden"),
    };
    let json = json!({
        "title": view.title,
        "detail": view.detail,
        "progress": progress,
        "busy": view.busy,
        "tone": if view.error { "error" } else { "normal" },
    })
    .to_string();
    if let Some(last) = app.try_state::<Last>() {
        *lock(&last.0) = Some(json.clone());
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.eval(script(&json));
    }
}

/// ウィンドウが出ているか (進み具合を渡すかどうか)。
pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some()
}

/// 少し待ってから閉じる (結果を読めるように)。
pub fn close_after(app: &AppHandle, delay: Duration) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.close();
        }
        if let Some(last) = app.try_state::<Last>() {
            *lock(&last.0) = None;
        }
    });
}

fn script(json: &str) -> String {
    // json は serde_json が作った文字列なので、そのまま JavaScript の式として使える。
    format!("window.zooUpdater && window.zooUpdater.set({json});")
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
