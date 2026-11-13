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
//! 利用者が `check_updates = false` (`ZOO_CHECK_UPDATES=false`) にしたときも、自動では確かめない
//! (TUI と同じ設定)。
//!
//! macOS では、メニューバーのアプリのメニューに「アップデートを確認…」を置く。押すと、その場で
//! 確かめ、新しい版があれば取得して更新し、再起動する。`check_updates = false` でも使える
//! (利用者が自分で押したときだけ確かめるので)。
//!
//! 利用者が更新を始めたとき (メニュー、トレイ、通知) は、更新のウィンドウ (`updater_window`) に
//! 確認、ダウンロードの進み具合、インストールを順に出す。自動の確認は裏で静かに行う。

use std::{sync::Mutex, time::Duration};

use tauri::{
    AppHandle, Manager, Wry,
    menu::{Menu, MenuItem},
    utils::platform::bundle_type,
};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    toast,
    updater_window::{self, Bar, View},
};

/// トレイのメニューの「更新して再起動」。
pub const MENU_ID: &str = "update";

/// macOS のメニューバーの「アップデートを確認…」。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const CHECK_MENU_ID: &str = "check_update";

/// 起動の直後はログインや同期で忙しいので、少し待ってから確かめる。
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// 受信が止まってから諦めるまでの時間。応答が止まっても、確認中のまま残らないように。
/// 全体の時間には上限を付けない (遅い回線でも、受信が進んでいれば数十 MB の取得を終えられるように)。
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 接続を確立するまでの上限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// トレイのメニュー。更新の準備ができたら、項目を足す。
pub struct TrayMenu(pub Menu<Wry>);

/// macOS のメニューバーの「アップデートを確認…」。更新の準備ができたら、文言を変える。
pub struct CheckMenuItem(pub MenuItem<Wry>);

#[derive(Default)]
struct Updates {
    /// 取得して署名を確かめた更新。
    ready: Mutex<Option<Ready>>,
    /// 足したメニューの項目。新しい版が続けて出たら、文言だけ変える。
    item: Mutex<Option<MenuItem<Wry>>>,
    /// 確かめている最中か (自動の確認と、メニューから押したときが重ならないように)。
    checking: Mutex<bool>,
    /// 取得が終わったら、すぐ入れて再起動する (自動の取得の最中にメニューから押したとき)。
    install_when_ready: Mutex<bool>,
}

struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

/// 更新の確認を始める。使えないビルドでは何もしない。利用者が止めたときは、自動では確かめない
/// (メニューの「アップデートを確認…」は使える)。
pub fn spawn(app: &AppHandle, check_updates: bool) {
    if !enabled(app) {
        return;
    }
    app.manage(Updates::default());
    if !check_updates {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            if let Err(err) = check(&app, true).await {
                eprintln!("更新の確認に失敗しました: {err}");
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// メニューバーの「アップデートを確認…」を押したとき。更新のウィンドウを出し、準備済みの
/// 更新があればすぐ入れる。なければその場で確かめ、新しい版があれば取得して入れる。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn check_now(app: &AppHandle) {
    updater_window::open(app);
    let Some(state) = app.try_state::<Updates>() else {
        show_result(
            app,
            "このビルドは更新できません",
            "アプリの中からの更新は、GitHub Releases の版だけが対応しています。",
            true,
        );
        return;
    };
    if lock(&state.ready).is_some() {
        install_and_restart(app);
        return;
    }
    updater_window::set(
        app,
        View {
            title: "更新を確かめています",
            detail: "",
            bar: Bar::Indeterminate,
            busy: true,
            error: false,
        },
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match check(&app, false).await {
            Ok(Checked::Ready) => install_and_restart(&app),
            // 自動の取得が進んでいる。終わったら入れる (進み具合はそちらがウィンドウに出す)。
            Ok(Checked::Busy) => {
                *lock(&app.state::<Updates>().install_when_ready) = true;
            }
            Ok(Checked::UpToDate) => {
                let version = app.package_info().version.to_string();
                show_result(&app, "zoo2 は最新です", &format!("v{version}"), false);
            }
            Err(err) => show_result(&app, "更新を確かめられませんでした", &err.to_string(), true),
        }
    });
}

/// 結果をウィンドウに出して、少し後に閉じる。
fn show_result(app: &AppHandle, title: &str, detail: &str, error: bool) {
    updater_window::set(
        app,
        View {
            title,
            detail,
            bar: Bar::Hidden,
            busy: false,
            error,
        },
    );
    let delay = if error { 8 } else { 3 };
    updater_window::close_after(app, Duration::from_secs(delay));
}

/// 確かめた結果。
enum Checked {
    /// 新しい版を取得して、署名を確かめた。
    Ready,
    /// 新しい版はない。
    UpToDate,
    /// ほかの確認が進んでいる。
    Busy,
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

/// 新しい版を確かめて、あれば取得する。`notify` のときは、準備ができたことを通知で知らせる
/// (メニューから押したときは、続けてすぐ入れるので知らせない)。
async fn check(app: &AppHandle, notify: bool) -> tauri_plugin_updater::Result<Checked> {
    let state = app.state::<Updates>();
    {
        let mut checking = lock(&state.checking);
        if *checking {
            return Ok(Checked::Busy);
        }
        *checking = true;
    }
    let result = fetch(app, notify).await;
    *lock(&state.checking) = false;
    result
}

async fn fetch(app: &AppHandle, notify: bool) -> tauri_plugin_updater::Result<Checked> {
    // configure_client は、確認とダウンロードの両方の通信に効く。
    let updater = app
        .updater_builder()
        .configure_client(|client| {
            client
                .read_timeout(READ_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
        })
        .build()?;
    let Some(update) = updater.check().await? else {
        return Ok(Checked::UpToDate);
    };
    let state = app.state::<Updates>();
    let already = lock(&state.ready)
        .as_ref()
        .is_some_and(|ready| ready.update.version == update.version);
    if already {
        return Ok(Checked::Ready);
    }
    let version = update.version.clone();
    // download は署名を確かめてから返す。更新のウィンドウが出ていれば、進み具合を出す。
    let mut progress = Progress::new(app, &version);
    let bytes = update
        .download(|chunk, total| progress.add(chunk, total), || {})
        .await?;
    *lock(&state.ready) = Some(Ready { update, bytes });

    show_menu_item(app, &version);
    // 自動の取得の最中にメニューから押されていたら、通知は出さずにそのまま入れる。
    if std::mem::take(&mut *lock(&state.install_when_ready)) {
        install_and_restart(app);
    } else if notify {
        toast::show_with(
            app,
            "zoo2 を更新できます",
            &format!("新しい版 v{version} の準備ができました。押すと、更新して再起動します。"),
            install_and_restart,
        );
    }
    Ok(Checked::Ready)
}

/// ダウンロードの進み具合を、更新のウィンドウに出す (出ていないときは何もしない)。
/// 表示を変えるのは、1% 進んだとき (大きさが分からないときは 1 MiB ごと) だけにする。
struct Progress {
    app: AppHandle,
    version: String,
    received: u64,
    /// 最後に表示した値 (% か MiB)。
    shown: Option<u64>,
}

impl Progress {
    fn new(app: &AppHandle, version: &str) -> Self {
        Self {
            app: app.clone(),
            version: version.to_string(),
            received: 0,
            shown: None,
        }
    }

    fn add(&mut self, chunk: usize, total: Option<u64>) {
        const MIB: u64 = 1024 * 1024;
        self.received += chunk as u64;
        if !updater_window::is_open(&self.app) {
            return;
        }
        let received_mib = self.received / MIB;
        let total = total.filter(|total| *total > 0);
        let (value, detail, bar) = match total {
            Some(total) => {
                let ratio = self.received as f64 / total as f64;
                let percent = (self.received * 100 / total).min(100);
                let detail = format!("{received_mib} / {} MiB ({percent}%)", total / MIB);
                (percent, detail, Bar::Ratio(ratio))
            }
            None => (
                received_mib,
                format!("{received_mib} MiB"),
                Bar::Indeterminate,
            ),
        };
        if self.shown == Some(value) {
            return;
        }
        self.shown = Some(value);
        let title = format!("v{} をダウンロードしています", self.version);
        updater_window::set(
            &self.app,
            View {
                title: &title,
                detail: &detail,
                bar,
                busy: true,
                error: false,
            },
        );
    }
}

fn show_menu_item(app: &AppHandle, version: &str) {
    let text = format!("更新して再起動 (v{version})");
    if let Some(check_item) = app.try_state::<CheckMenuItem>() {
        let _ = check_item.0.set_text(format!("{text}…"));
    }
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
    updater_window::open(app);
    let title = format!("v{} をインストールしています", ready.update.version);
    updater_window::set(
        app,
        View {
            title: &title,
            detail: "終わると、zoo2 を再起動します",
            bar: Bar::Indeterminate,
            busy: true,
            error: false,
        },
    );
    let app = app.clone();
    // deb では pkexec が管理者のパスワードを聞くので、待つ間に他を止めないよう別のスレッドで入れる。
    std::thread::spawn(move || {
        let result = ready.update.install(&ready.bytes);
        match result {
            Ok(()) => app.restart(),
            Err(err) => {
                // パスワードの入力をやめたときなども、もう一度押せるように残す。
                *lock(&app.state::<Updates>().ready) = Some(ready);
                show_result(&app, "更新できませんでした", &err.to_string(), true);
            }
        }
    });
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
