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

/// 最後に渡した状態 (読み込みが終わったときに渡し直す) と、その世代。
/// 世代はウィンドウを出すときと状態を渡すときに増やし、閉じる予約は、予約したときの世代の
/// ままのときだけ閉じる (結果を出した後にもう一度押して始めた作業の最中に、前の予約で閉じない
/// ように)。世代を増やすのと閉じるのは同じロックの中で行う。
#[derive(Default)]
struct Last(Mutex<LastState>);

#[derive(Default)]
struct LastState {
    json: Option<String>,
    generation: u64,
}

/// ウィンドウを出す (出ていれば前に出す)。新しい作業を始めるときに呼ぶ。
pub fn open(app: &AppHandle) {
    if app.try_state::<Last>().is_none() {
        app.manage(Last::default());
    }
    let Some(last) = app.try_state::<Last>() else {
        return;
    };
    // ロックを持ったまま世代を増やして出す。前の閉じる予約は、この前に隠し終えているか、
    // この後に世代の違いを見て何もしないかのどちらかになる。
    {
        let mut last = lock(&last.0);
        last.generation += 1;
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
    }
    // まだ作っていない (閉じる予約が触るウィンドウもない)。読み込みが終わったときの
    // `on_page_load` が同じロックを取るので、ロックを放してから作る。
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
                    && let Some(json) = lock(&last.0).json.clone()
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
        let mut last = lock(&last.0);
        last.json = Some(json.clone());
        last.generation += 1;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.eval(script(&json));
    }
}

/// ウィンドウが出ているか (進み具合を渡すかどうか)。
pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .is_some_and(|window| window.is_visible().unwrap_or(false))
}

/// 少し待ってから閉じる (結果を読めるように)。その間にウィンドウを出し直すか新しい状態を
/// 渡したら、閉じない。
///
/// 閉じるのではなく隠す。`close` はウィンドウを後から破棄するので、直後の `open` が
/// 破棄される途中のウィンドウを前に出してしまうことがある。隠すのと出すのは、同じイベント
/// ループに順に届くので、後から出したほうが残る。
pub fn close_after(app: &AppHandle, delay: Duration) {
    let Some(generation) = app.try_state::<Last>().map(|last| lock(&last.0).generation) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let Some(last) = app.try_state::<Last>() else {
            return;
        };
        let last = lock(&last.0);
        if last.generation != generation {
            return;
        }
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.hide();
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
