//! アプリの自動更新 (docs/desktop-tauri.md の「更新」)。
//!
//! GitHub Releases の `latest.json` を、起動の少し後と 6 時間ごとに確かめる。新しい版があれば
//! 裏で取得して署名を確かめておき、トレイのメニューと通知で知らせる。入れ替えと再起動は、
//! 利用者がメニューか通知を押したときだけ行う (書きかけの下書きなどを勝手に失わないため)。
//!
//! 公開鍵 (`plugins.updater.pubkey`) は、リリースの workflow がビルドのときに入れる。空のとき
//! (ローカルや CI のビルド) は何もしない。Linux では、deb や AppImage から動いていないとき
//! (`bundle_type()` が `None`。`cargo tauri dev` など) も何もしない。macOS の `bundle_type()` は
//! 印がなくても `App` を返すので、公開鍵の有無だけで決まる (公開鍵はリリースの `.app` にだけ入る)。
//! 利用者が `check_updates = false` (`ZOO_CHECK_UPDATES=false`) にしたときも、確かめない (TUI と同じ設定)。

use std::{sync::Mutex, time::Duration};

use tauri::{
    AppHandle, Manager, Wry,
    menu::{Menu, MenuItem},
    utils::platform::bundle_type,
};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::toast;

/// トレイのメニューの「更新して再起動」。
pub const MENU_ID: &str = "update";

/// 起動の直後はログインや同期で忙しいので、少し待ってから確かめる。
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// トレイのメニュー。更新の準備ができたら、項目を足す。
pub struct TrayMenu(pub Menu<Wry>);

#[derive(Default)]
struct Updates {
    /// 取得して署名を確かめた更新。
    ready: Mutex<Option<Ready>>,
    /// 足したメニューの項目。新しい版が続けて出たら、文言だけ変える。
    item: Mutex<Option<MenuItem<Wry>>>,
}

struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

/// 更新の確認を始める。使えないビルドと、利用者が止めたときは何もしない。
pub fn spawn(app: &AppHandle, check_updates: bool) {
    if !check_updates || !enabled(app) {
        return;
    }
    app.manage(Updates::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            if let Err(err) = check(&app).await {
                eprintln!("更新の確認に失敗しました: {err}");
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

fn enabled(app: &AppHandle) -> bool {
    let pubkey = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|pubkey| pubkey.as_str())
        .unwrap_or_default();
    !pubkey.is_empty() && bundle_type().is_some()
}

async fn check(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    let state = app.state::<Updates>();
    let already = lock(&state.ready)
        .as_ref()
        .is_some_and(|ready| ready.update.version == update.version);
    if already {
        return Ok(());
    }
    // download は署名を確かめてから返す。
    let bytes = update.download(|_, _| {}, || {}).await?;
    let version = update.version.clone();
    *lock(&state.ready) = Some(Ready { update, bytes });

    show_menu_item(app, &version);
    toast::show_with(
        app,
        "zoo2 を更新できます",
        &format!("新しい版 v{version} の準備ができました。押すと、更新して再起動します。"),
        install_and_restart,
    );
    Ok(())
}

fn show_menu_item(app: &AppHandle, version: &str) {
    let text = format!("更新して再起動 (v{version})");
    let state = app.state::<Updates>();
    let mut item = lock(&state.item);
    if let Some(item) = item.as_ref() {
        let _ = item.set_text(text);
        return;
    }
    let Some(menu) = app.try_state::<TrayMenu>() else {
        return;
    };
    if let Ok(new) = MenuItem::with_id(app, MENU_ID, text, true, None::<&str>)
        && menu.0.prepend(&new).is_ok()
    {
        *item = Some(new);
    }
}

/// 準備した更新を入れて、再起動する。
pub fn install_and_restart(app: &AppHandle) {
    let Some(state) = app.try_state::<Updates>() else {
        return;
    };
    let Some(ready) = lock(&state.ready).take() else {
        return;
    };
    let app = app.clone();
    // deb では pkexec が管理者のパスワードを聞くので、待つ間に他を止めないよう別のスレッドで入れる。
    std::thread::spawn(move || {
        let result = ready.update.install(&ready.bytes);
        match result {
            Ok(()) => app.restart(),
            Err(err) => {
                // パスワードの入力をやめたときなども、もう一度押せるように残す。
                *lock(&app.state::<Updates>().ready) = Some(ready);
                toast::show_with(
                    &app,
                    "zoo2 を更新できませんでした",
                    &err.to_string(),
                    |_| {},
                );
            }
        }
    });
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
