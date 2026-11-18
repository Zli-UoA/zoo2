//! アプリの自動更新 (docs/desktop-tauri.md の「更新」)。
//!
//! GitHub Releases の `latest.json` を、起動の少し後と 6 時間ごとに確かめる。新しい版があれば
//! 裏で取得して署名を確かめておき、トレイのメニューと通知で知らせる。入れ替えと再起動は、
//! 利用者がメニューか通知を押したときだけ行う (書きかけの下書きなどを勝手に失わないため)。
//!
//! 公開鍵 (`plugins.updater.pubkey`) は、リリースの workflow がビルドのときに入れる。空のとき
//! (ローカルや CI のビルド) は何もしない。Linux では、deb や AppImage から動いていないとき
//! (`bundle_type()` が `None`。`cargo tauri dev` など) も何もしない。Windows では Inno Setup で
//! インストールされた環境 (`unins000.exe` の存在) か `bundle_type()` で判定する。macOS の
//! `bundle_type()` は印がなくても `App` を返すので、公開鍵の有無だけで決まる (公開鍵はリリースの
//! `.app` にだけ入る)。利用者が `check_updates = false` (`ZOO_CHECK_UPDATES=false`) にしたときも、
//! 自動では確かめない (TUI と同じ設定)。
//!
//! macOS ではメニューバーのアプリのメニューに、Windows・Linux ではトレイメニューに
//! 「アップデートを確認…」を置く。押すと、その場で確かめ、新しい版があれば取得して更新し、
//! 再起動する。`check_updates = false` でも使える (利用者が自分で押したときだけ確かめるので)。
//!
//! 利用者が更新を始めたとき (メニュー、トレイ、通知) は、更新のウィンドウ (`updater_window`) に
//! 確認、ダウンロードの進み具合、インストールを順に出す。自動の確認は裏で静かに行う。

use std::{sync::Mutex, time::Duration};

#[cfg(target_os = "macos")]
use tauri::Wry;
#[cfg(target_os = "macos")]
use tauri::menu::Menu;
use tauri::{AppHandle, Manager, menu::MenuItem, utils::platform::bundle_type};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    toast,
    updater_window::{self, Bar, View},
};

/// macOS トレイのメニューの「更新して再起動」。
#[cfg(target_os = "macos")]
pub const MENU_ID: &str = "update";

/// メニューバー (macOS) またはトレイ (Windows・Linux) の「アップデートを確認…」。
pub const CHECK_MENU_ID: &str = "check_update";

/// 起動の直後はログインや同期で忙しいので、少し待ってから確かめる。
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// 受信が止まってから諦めるまでの時間。応答が止まっても、確認中のまま残らないように。
/// 全体の時間には上限を付けない (遅い回線でも、受信が進んでいれば数十 MB の取得を終えられるように)。
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 接続を確立するまでの上限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// トレイのメニュー。macOS で更新の準備ができたら、項目を足す。
#[cfg(target_os = "macos")]
pub struct TrayMenu(pub Menu<Wry>);

/// メニューバー (macOS) またはトレイ (Windows・Linux) の「アップデートを確認…」。
/// 更新の準備ができたら、文言を変える。
pub struct CheckMenuItem(pub MenuItem<tauri::Wry>);

#[derive(Default)]
struct Updates {
    /// 取得して署名を確かめた更新。
    ready: Mutex<Option<Ready>>,
    /// インストーラーの配置や起動を進めている最中か (同時実行で同じ一時ファイルを壊さないため)。
    installing: Mutex<bool>,
    /// macOS のトレイへ足したメニューの項目。新しい版が続けて出たら、文言だけ変える。
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "windows")]
    {
        cleanup_old_inno_installers(app);
        if let Some(detail) = take_inno_failure_message(std::env::args_os().skip(1)) {
            show_inno_failure(app, &detail);
        }
    }

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

/// メニューバーやトレイの「アップデートを確認…」を押したとき。更新のウィンドウを出し、
/// 準備済みの更新があればすぐ入れる。なければその場で確かめ、新しい版があれば取得して入れる。
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
            // 自動の確認が進んでいる。check が予約したので、その確認が終わったときに
            // 結果を出す (新しい版なら入れる)。進み具合もそちらがウィンドウに出す。
            Ok(Checked::Busy) => {}
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
    if pubkey.is_empty() {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        bundle_type().is_some() || is_inno_installed()
    }
    #[cfg(not(target_os = "windows"))]
    {
        bundle_type().is_some()
    }
}

#[cfg(target_os = "windows")]
const INNO_UNINSTALL_REGKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{8E9B3B8E-3B89-4F0F-B7C5-6E4F1D79A2C1}_is1";

#[cfg(target_os = "windows")]
fn is_inno_installed() -> bool {
    cfg!(not(debug_assertions))
        && std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(std::path::Path::parent)
            .is_some_and(has_matching_inno_registry)
}

#[cfg(target_os = "windows")]
fn has_matching_inno_registry(dir: &std::path::Path) -> bool {
    use windows::{
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ,
                REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
            },
        },
        core::PCWSTR,
    };

    struct RegKey(HKEY);
    impl Drop for RegKey {
        fn drop(&mut self) {
            // SAFETY: Open に成功したハンドルを閉じる。
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn read_sz(hkey: HKEY, name: &str) -> Option<String> {
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut kind = REG_VALUE_TYPE::default();
        let mut byte_len: u32 = 0;
        // SAFETY: 値の種類とサイズを問い合わせる。
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name_wide.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut byte_len),
            )
        };
        if status != ERROR_SUCCESS
            || (kind != REG_SZ && kind != REG_EXPAND_SZ)
            || byte_len == 0
            || !byte_len.is_multiple_of(2)
            || byte_len > 64 * 1024
        {
            return None;
        }
        let mut buf = vec![0u16; (byte_len / 2) as usize];
        // SAFETY: 十分な長さの UTF-16 バッファへ文字列値を読み取る。
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name_wide.as_ptr()),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr().cast::<u8>()),
                Some(&mut byte_len),
            )
        };
        if status != ERROR_SUCCESS || !byte_len.is_multiple_of(2) {
            return None;
        }
        let words = ((byte_len / 2) as usize).min(buf.len());
        let slice = &buf[..words];
        let trimmed_len = slice
            .iter()
            .rposition(|&c| c != 0)
            .map_or(0, |last| last + 1);
        String::from_utf16(&slice[..trimmed_len]).ok()
    }

    let key_wide: Vec<u16> = INNO_UNINSTALL_REGKEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut raw = HKEY::default();
        // SAFETY: NUL 終端済みキー名を読み取り専用で開く。
        let status =
            unsafe { RegOpenKeyExW(root, PCWSTR(key_wide.as_ptr()), Some(0), KEY_READ, &mut raw) };
        if status != ERROR_SUCCESS {
            continue;
        }
        let guard = RegKey(raw);
        let install_location = read_sz(guard.0, "InstallLocation");
        let uninstall_string = read_sz(guard.0, "UninstallString");
        if matches_inno_uninstall_entry_on_disk(
            dir,
            install_location.as_deref(),
            uninstall_string.as_deref(),
        ) {
            return true;
        }
    }
    false
}

#[cfg(any(target_os = "windows", test))]
fn normalize_windows_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').replace('/', "\\");
    let without_verbatim = if let Some(unc) = trimmed
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| trimmed.strip_prefix(r"\\?\unc\"))
    {
        format!(r"\\{unc}")
    } else if let Some(local) = trimmed.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        trimmed
    };
    without_verbatim.trim_end_matches('\\').to_ascii_lowercase()
}

#[cfg(any(target_os = "windows", test))]
fn canonicalize_if_exists(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(any(target_os = "windows", test))]
fn extract_exe_from_command(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        trimmed.split_whitespace().next().unwrap_or("")
    }
}

#[cfg(any(target_os = "windows", test))]
fn is_inno_uninstaller_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("unins") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".exe") else {
        return false;
    };
    digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(any(target_os = "windows", test))]
fn split_uninstaller_parent_and_name(raw: &str) -> Option<(&str, &str)> {
    let trimmed = raw.trim().trim_matches('"');
    let sep_idx = trimmed.rfind(['\\', '/'])?;
    let file_name = &trimmed[sep_idx + 1..];
    if file_name.is_empty() {
        return None;
    }
    let parent = if sep_idx == 0 {
        &trimmed[..1]
    } else if sep_idx == 2 && trimmed.as_bytes()[1] == b':' {
        &trimmed[..=sep_idx]
    } else {
        &trimmed[..sep_idx]
    };
    Some((parent, file_name))
}

#[cfg(any(target_os = "windows", test))]
fn matches_inno_uninstall_entry(
    exe_dir: &std::path::Path,
    install_location: Option<&str>,
    uninstall_string: Option<&str>,
) -> Option<std::path::PathBuf> {
    let canonical_dir = canonicalize_if_exists(exe_dir);
    let dir_norm = normalize_windows_path(&canonical_dir.to_string_lossy());
    if dir_norm.is_empty() {
        return None;
    }
    let location_matches = install_location
        .map(|loc| {
            let trimmed = loc.trim().trim_matches('"');
            let resolved = canonicalize_if_exists(std::path::Path::new(trimmed));
            normalize_windows_path(&resolved.to_string_lossy())
        })
        .is_some_and(|loc| loc == dir_norm);
    if !location_matches {
        return None;
    }

    let raw_uninstaller = extract_exe_from_command(uninstall_string?);
    let (raw_parent, file_name) = split_uninstaller_parent_and_name(raw_uninstaller)?;
    if !is_inno_uninstaller_name(file_name) {
        return None;
    }
    let resolved_parent = canonicalize_if_exists(std::path::Path::new(raw_parent));
    if normalize_windows_path(&resolved_parent.to_string_lossy()) != dir_norm {
        return None;
    }
    Some(canonical_dir.join(file_name))
}

#[cfg(any(target_os = "windows", test))]
fn matches_inno_uninstall_entry_on_disk(
    exe_dir: &std::path::Path,
    install_location: Option<&str>,
    uninstall_string: Option<&str>,
) -> bool {
    matches_inno_uninstall_entry(exe_dir, install_location, uninstall_string)
        .is_some_and(|uninstaller| uninstaller.is_file())
}

/// 新しい版を確かめて、あれば取得する。`notify` のときは、準備ができたことを通知で知らせる
/// (メニューから押したときは、続けてすぐ入れるので知らせない)。
///
/// メニューから押したとき (`notify` が false) にほかの確認が進んでいたら、その確認に結果の表示を
/// 予約して `Busy` を返す。予約した確認は、終わったときに (新しい版、更新なし、失敗のどれでも)
/// 更新のウィンドウに結果を出し、予約を消す。予約と確認中の印は同じロックの中で読み書きする
/// (確認が終わる瞬間に予約して、予約が残り続けることがないように)。
async fn check(app: &AppHandle, notify: bool) -> tauri_plugin_updater::Result<Checked> {
    let state = app.state::<Updates>();
    {
        let mut checking = lock(&state.checking);
        if *checking {
            if !notify {
                *lock(&state.install_when_ready) = true;
            }
            return Ok(Checked::Busy);
        }
        *checking = true;
    }
    let result = fetch(app, notify).await;
    let requested = {
        let mut checking = lock(&state.checking);
        *checking = false;
        std::mem::take(&mut *lock(&state.install_when_ready))
    };
    if requested {
        match &result {
            Ok(Checked::Ready) => install_and_restart(app),
            Ok(Checked::UpToDate) => {
                let version = app.package_info().version.to_string();
                show_result(app, "zoo2 は最新です", &format!("v{version}"), false);
            }
            Ok(Checked::Busy) => {}
            Err(err) => show_result(app, "更新を確かめられませんでした", &err.to_string(), true),
        }
    }
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
    // メニューから押されて予約されていたら、通知は出さない (check がそのまま入れる)。
    let requested = *lock(&state.install_when_ready);
    if notify && !requested {
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
        #[cfg(target_os = "macos")]
        let _ = check_item.0.set_text(format!("{text}…"));
        #[cfg(not(target_os = "macos"))]
        let _ = check_item.0.set_text(&text);
    }
    #[cfg(target_os = "macos")]
    {
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
}

/// 準備した更新を入れて、再起動する。
pub fn install_and_restart(app: &AppHandle) {
    let Some(state) = app.try_state::<Updates>() else {
        return;
    };
    let mut installing = lock(&state.installing);
    if *installing {
        return;
    }
    let Some(ready) = lock(&state.ready).take() else {
        return;
    };
    *installing = true;
    drop(installing);
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
    // deb では pkexec が、Windows ではインストーラー起動や UAC が待つことがあるため別スレッドで実行する。
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        if bundle_type().is_none() {
            match install_inno_setup(&app, &ready) {
                Ok(()) => app.exit(0),
                Err(err) => finish_failed_install(&app, ready, &err.to_string()),
            }
            return;
        }
        let result = ready.update.install(&ready.bytes);
        match result {
            Ok(()) => app.restart(),
            Err(err) => {
                // パスワードの入力をやめたときなども、もう一度押せるように残す。
                // インストール中に新しい版の取得が終わっていた場合は、古い版で上書きしない。
                finish_failed_install(&app, ready, &err.to_string());
            }
        }
    });
}

fn finish_failed_install(app: &AppHandle, failed: Ready, detail: &str) {
    let updates = app.state::<Updates>();
    *lock(&updates.installing) = false;
    restore_ready_on_failure(app, failed);
    show_result(app, "更新できませんでした", detail, true);
}

fn restore_ready_on_failure(app: &AppHandle, failed: Ready) {
    let updates = app.state::<Updates>();
    let mut guard = lock(&updates.ready);
    if let Some(existing) = guard.as_ref()
        && should_keep_existing_version(&existing.update.version, &failed.update.version)
    {
        return;
    }
    *guard = Some(failed);
}

#[derive(Debug, Eq, PartialEq)]
enum PreIdent<'a> {
    Num(u64),
    Text(&'a str),
}

impl Ord for PreIdent<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
            (Self::Num(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Num(_)) => std::cmp::Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreIdent<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64, Vec<PreIdent<'_>>)> {
    let core_and_pre = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    let without_build = core_and_pre.split('+').next()?;
    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some((_, _)) => return None,
        None => (without_build, None),
    };
    let mut nums = core.split('.');
    let major = nums.next()?.parse::<u64>().ok()?;
    let minor = nums.next()?.parse::<u64>().ok()?;
    let patch = nums.next()?.parse::<u64>().ok()?;
    if nums.next().is_some() {
        return None;
    }
    let pre_idents = match pre {
        None => Vec::new(),
        Some(pre) => pre
            .split('.')
            .map(|part| match part.parse::<u64>() {
                Ok(n) => PreIdent::Num(n),
                Err(_) => PreIdent::Text(part),
            })
            .collect(),
    };
    Some((major, minor, patch, pre_idents))
}

fn compare_semver(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let (amaj, amin, apatch, apre) = parse_semver(a)?;
    let (bmaj, bmin, bpatch, bpre) = parse_semver(b)?;
    let core_ord = (amaj, amin, apatch).cmp(&(bmaj, bmin, bpatch));
    if core_ord != std::cmp::Ordering::Equal {
        return Some(core_ord);
    }
    Some(match (apre.is_empty(), bpre.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => apre.cmp(&bpre),
    })
}

fn should_keep_existing_version(existing: &str, failed: &str) -> bool {
    match compare_semver(existing, failed) {
        Some(ord) => ord != std::cmp::Ordering::Less,
        // 既に最新の確認で取得された値を残す方が安全。
        None => true,
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn take_inno_failure_message<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg.as_ref() == "--update-failed" {
            let code = iter.next().and_then(|s| {
                s.as_ref()
                    .to_str()
                    .and_then(|t| t.trim().parse::<i32>().ok())
            });
            return Some(match code {
                Some(-1) | None => "インストーラーを起動または実行できませんでした".to_string(),
                Some(code) => format!("インストーラーが終了しました (コード {code})"),
            });
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub fn show_inno_failure(app: &AppHandle, detail: &str) {
    updater_window::open(app);
    show_result(app, "更新できませんでした", detail, true);
}

#[cfg(any(target_os = "windows", test))]
fn inno_installer_args(install_dir: &std::path::Path, pid: u32) -> String {
    let dir = install_dir
        .to_string_lossy()
        .replace('"', "")
        .trim_end_matches('\\')
        .to_string();
    format!(
        r#"/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /DIR="{dir}" /WAITPID={pid} /RELAUNCH=1"#
    )
}

#[cfg(target_os = "windows")]
fn updater_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join("updates"))
        .map_err(|err| format!("更新ファイルの保存先を取得できませんでした ({err})"))
}

#[cfg(any(target_os = "windows", test))]
const UPDATER_CLEANUP_GRACE: Duration = Duration::from_secs(15 * 60);

#[cfg(any(target_os = "windows", test))]
fn is_stale_updater_artifact(
    modified: Option<std::time::SystemTime>,
    now: std::time::SystemTime,
    lock_active: bool,
) -> bool {
    if lock_active {
        return false;
    }
    let Some(modified) = modified else {
        return false;
    };
    now.duration_since(modified)
        .is_ok_and(|age| age >= UPDATER_CLEANUP_GRACE)
}

#[cfg(target_os = "windows")]
fn try_lock_updater_exclusive(lock_path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(lock_path)
        .ok()
}

#[cfg(target_os = "windows")]
fn cleanup_old_inno_installers(app: &AppHandle) {
    let Ok(dir) = updater_dir(app) else {
        return;
    };
    cleanup_old_inno_installers_in(&dir, std::time::SystemTime::now());
}

#[cfg(target_os = "windows")]
fn cleanup_old_inno_installers_in(dir: &std::path::Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        if name.starts_with("zoo2-updater-")
            && (name.ends_with(".exe") || name.ends_with(".exe.part"))
        {
            if !is_stale_updater_artifact(meta.modified().ok(), now, false) {
                continue;
            }
            let base_name = name.strip_suffix(".part").unwrap_or(name);
            let lock_path = updater_lock_path(&dir.join(base_name));
            // 排他ロックを保持したまま削除可否を判定し、取得直前の競合を防ぐ。
            let Some(guard) = try_lock_updater_exclusive(&lock_path) else {
                continue;
            };
            let still_stale = std::fs::symlink_metadata(&path)
                .ok()
                .is_some_and(|m| is_stale_updater_artifact(m.modified().ok(), now, false));
            if still_stale {
                let _ = std::fs::remove_file(&path);
            }
            drop(guard);
            let _ = std::fs::remove_file(&lock_path);
        } else if name.starts_with("zoo2-updater-")
            && name.ends_with(".exe.lock")
            && is_stale_updater_artifact(meta.modified().ok(), now, false)
        {
            let base = &name[..name.len() - ".lock".len()];
            if !dir.join(base).exists()
                && !dir.join(format!("{base}.part")).exists()
                && let Some(guard) = try_lock_updater_exclusive(&path)
            {
                drop(guard);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn updater_lock_path(installer_path: &std::path::Path) -> std::path::PathBuf {
    let mut os = installer_path.as_os_str().to_os_string();
    os.push(".lock");
    std::path::PathBuf::from(os)
}

#[cfg(target_os = "windows")]
fn updater_part_path(installer_path: &std::path::Path) -> std::path::PathBuf {
    let mut os = installer_path.as_os_str().to_os_string();
    os.push(".part");
    std::path::PathBuf::from(os)
}

#[cfg(any(target_os = "windows", test))]
const INNO_WATCHER_SCRIPT: &str = "\
$ErrorActionPreference = 'SilentlyContinue'\n\
$installer = $env:ZOO2_UPDATER_INSTALLER\n\
$exe = $env:ZOO2_UPDATER_EXE\n\
$argsLine = $env:ZOO2_UPDATER_ARGS\n\
$lockPath = $env:ZOO2_UPDATER_LOCK\n\
$oldPid = 0\n\
[void][int]::TryParse($env:ZOO2_UPDATER_PID, [ref]$oldPid)\n\
$lockStream = $null\n\
$relaunchOnFailure = $false\n\
$code = -1\n\
try {\n\
  if ($lockPath) {\n\
    $lockStream = [System.IO.File]::Open($lockPath, [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)\n\
  }\n\
  [Console]::Out.WriteLine('LOCKED')\n\
  [Console]::Out.Flush()\n\
  $waited = 0\n\
  while ($installer -and -not (Test-Path -LiteralPath $installer) -and $waited -lt 100) {\n\
    Start-Sleep -Milliseconds 50\n\
    $waited++\n\
  }\n\
  if (-not ($installer -and (Test-Path -LiteralPath $installer))) {\n\
    throw 'installer missing'\n\
  }\n\
  $relaunchOnFailure = $true\n\
  [Console]::Out.WriteLine('ARMED')\n\
  [Console]::Out.Flush()\n\
  if ($oldPid -gt 0) {\n\
    $old = Get-Process -Id $oldPid -ErrorAction SilentlyContinue\n\
    if ($old -and $old.ProcessName -like 'zoo2*') { $null = $old.WaitForExit(15000) }\n\
  }\n\
  $p = Start-Process -FilePath $installer -ArgumentList $argsLine -PassThru\n\
  if ($p) {\n\
    $null = $p.Handle\n\
    $p.WaitForExit()\n\
    $code = [int]$p.ExitCode\n\
  }\n\
} catch {\n\
  $code = -1\n\
} finally {\n\
  if ($installer) { Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue }\n\
  if ($lockStream) { $lockStream.Dispose() }\n\
  if ($lockPath) { Remove-Item -LiteralPath $lockPath -Force -ErrorAction SilentlyContinue }\n\
}\n\
if ($relaunchOnFailure -and $code -ne 0 -and $exe -and (Test-Path -LiteralPath $exe)) {\n\
  Start-Process -FilePath $exe -ArgumentList @('--update-failed', [string]$code)\n\
}\n";

#[cfg(any(target_os = "windows", test))]
fn encode_utf16le_base64(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(target_os = "windows")]
fn system_powershell_exe() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .map(|root| root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("powershell.exe"))
}

#[cfg(target_os = "windows")]
type WatcherStdoutRx = std::sync::mpsc::Receiver<std::io::Result<String>>;

#[cfg(target_os = "windows")]
fn await_watcher_signal(
    child: &mut std::process::Child,
    rx: &WatcherStdoutRx,
    expected: &str,
) -> Result<(), String> {
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(line)) if line.trim() == expected => Ok(()),
        Ok(Ok(line)) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!(
                "更新監視プロセスの初期化に失敗しました ({})",
                line.trim()
            ))
        }
        Ok(Err(err)) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("更新監視プロセスの待機に失敗しました ({err})"))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!(
                "更新監視プロセスのシグナル ({expected}) 待機がタイムアウトしました"
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn spawn_locked_powershell_watcher(
    script: &str,
    old_pid: u32,
    installer_path: &std::path::Path,
    exe_path: &std::path::Path,
    args_line: &str,
    lock_path: &std::path::Path,
) -> Result<(std::process::Child, WatcherStdoutRx), String> {
    use std::{
        io::{BufRead, BufReader},
        os::windows::process::CommandExt,
        process::Stdio,
        sync::mpsc,
    };
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let encoded = encode_utf16le_base64(script);
    let mut child = std::process::Command::new(system_powershell_exe())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &encoded,
        ])
        .env("ZOO2_UPDATER_PID", old_pid.to_string())
        .env("ZOO2_UPDATER_INSTALLER", installer_path)
        .env("ZOO2_UPDATER_EXE", exe_path)
        .env("ZOO2_UPDATER_ARGS", args_line)
        .env("ZOO2_UPDATER_LOCK", lock_path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|err| format!("インストーラーを起動できませんでした ({err})"))?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("更新監視プロセスの出力を取得できませんでした".to_string());
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                    break;
                }
            }
        }
    });
    await_watcher_signal(&mut child, &rx, "LOCKED")?;
    Ok((child, rx))
}

#[cfg(target_os = "windows")]
fn install_inno_setup(app: &AppHandle, ready: &Ready) -> Result<(), String> {
    ready
        .update
        .verify(&ready.bytes)
        .map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let install_dir = exe
        .parent()
        .ok_or_else(|| "インストール先ディレクトリが見つかりません".to_string())?;

    let dir = updater_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let version: String = ready
        .update
        .version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let installer_path = dir.join(format!("zoo2-updater-{version}-{}.exe", std::process::id()));
    let part_path = updater_part_path(&installer_path);
    let lock_path = updater_lock_path(&installer_path);
    std::fs::write(&part_path, &ready.bytes).map_err(|e| e.to_string())?;

    let args = inno_installer_args(install_dir, std::process::id());
    // 1) "LOCKED": 清掃競合を防ぐ排他ロック確保を確認してから .exe へ改名する。
    // 2) "ARMED": 監視プロセスが .exe を認識して起動待ちに入ったことを確認してから自プロセスを終了する。
    let (mut watcher, rx) = match spawn_locked_powershell_watcher(
        INNO_WATCHER_SCRIPT,
        std::process::id(),
        &installer_path,
        &exe,
        &args,
        &lock_path,
    ) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = std::fs::remove_file(&part_path);
            let _ = std::fs::remove_file(&lock_path);
            return Err(err);
        }
    };
    if let Err(err) = std::fs::rename(&part_path, &installer_path) {
        let _ = std::fs::remove_file(&part_path);
        let _ = watcher.kill();
        let _ = watcher.wait();
        let _ = std::fs::remove_file(&lock_path);
        return Err(format!("インストーラーを配置できませんでした ({err})"));
    }
    if let Err(err) = await_watcher_signal(&mut watcher, &rx, "ARMED") {
        let _ = std::fs::remove_file(&installer_path);
        let _ = std::fs::remove_file(&part_path);
        let _ = std::fs::remove_file(&lock_path);
        return Err(err);
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inno_registry_entry_requires_matching_location_and_uninstaller() {
        let dir = std::path::Path::new(r"C:\Users\Alice\AppData\Local\Programs\zoo2");
        assert!(
            matches_inno_uninstall_entry(
                dir,
                Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
                Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\unins000.exe""#),
            )
            .is_some()
        );
        assert!(
            matches_inno_uninstall_entry(
                dir,
                Some(r"c:/users/alice/appdata/local/programs/zoo2"),
                Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\UNINS001.EXE /SILENT"),
            )
            .is_some()
        );
        assert!(
            matches_inno_uninstall_entry(
                dir,
                Some(r"C:\Users\Bob\Programs\zoo2\"),
                Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\unins000.exe""#),
            )
            .is_none()
        );
        assert!(
            matches_inno_uninstall_entry(
                dir,
                Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
                Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\other.exe""#),
            )
            .is_none()
        );
        assert!(
            matches_inno_uninstall_entry(
                dir,
                Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
                None,
            )
            .is_none()
        );
        assert!(
            matches_inno_uninstall_entry(
                std::path::Path::new(r"\\?\C:\Users\Alice\AppData\Local\Programs\zoo2"),
                Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
                Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\unins000.exe""#),
            )
            .is_some()
        );
        assert_eq!(
            normalize_windows_path(r"\\?\UNC\server\share\zoo2\"),
            r"\\server\share\zoo2"
        );
    }

    #[test]
    fn inno_registry_entry_on_disk_accepts_unins000_and_unins001() {
        let base = std::env::temp_dir().join(format!("zoo2-inno-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let loc = base.to_string_lossy().to_string();

        // unins001.exe のみ存在する場合でも UninstallString に記載があれば許可する
        let unins001 = base.join("unins001.exe");
        assert!(!matches_inno_uninstall_entry_on_disk(
            &base,
            Some(&loc),
            Some(&format!(r#""{}" /SILENT"#, unins001.display())),
        ));
        std::fs::write(&unins001, b"").unwrap();
        assert!(matches_inno_uninstall_entry_on_disk(
            &base,
            Some(&loc),
            Some(&format!(r#""{}" /SILENT"#, unins001.display())),
        ));

        // unins000.exe が後から消えても、zoo2 自身の unins001.exe が一致すれば有効
        let unins000 = base.join("unins000.exe");
        assert!(!matches_inno_uninstall_entry_on_disk(
            &base,
            Some(&loc),
            Some(&format!(r#""{}""#, unins000.display())),
        ));
        std::fs::write(&unins000, b"").unwrap();
        assert!(matches_inno_uninstall_entry_on_disk(
            &base,
            Some(&loc),
            Some(&format!(r#""{}""#, unins000.display())),
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn inno_watcher_executes_ascii_and_smart_quote_paths_and_protects_from_cleanup() {
        let root = std::env::temp_dir().join(format!("zoo2-ps-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // Start-Process が扱う実体と同じ GUI .exe スタブを 1 回だけ生成し、各テストパスへ複製して検証する。
        let stub_exe = root.join("zoo2-stub.exe");
        let build_stub = "\
$ErrorActionPreference = 'Stop'\n\
$src = @'\n\
using System;\n\
using System.IO;\n\
using System.Reflection;\n\
using System.Text;\n\
public class Zoo2Stub {\n\
    public static int Main(string[] args) {\n\
        string self = Assembly.GetExecutingAssembly().Location;\n\
        string[] cfg = File.ReadAllLines(self + \".cfg\", Encoding.UTF8);\n\
        File.WriteAllText(cfg[0], Environment.CommandLine, Encoding.UTF8);\n\
        return int.Parse(cfg[1]);\n\
    }\n\
}\n\
'@\n\
Add-Type -TypeDefinition $src -OutputAssembly $env:ZOO2_STUB_OUT -OutputType WindowsApplication\n";
        let build_status = std::process::Command::new(system_powershell_exe())
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encode_utf16le_base64(build_stub),
            ])
            .env("ZOO2_STUB_OUT", &stub_exe)
            .status()
            .unwrap();
        assert!(build_status.success(), "failed to compile test stub exe");

        for (label, code, should_relaunch) in [
            ("O'Connor-ok", 0, false),
            ("O\u{2019}Connor-fail", 5, true),
            ("O'Connor-fail", 5, true),
            ("O\u{2019}Connor-ok", 0, false),
        ] {
            let case_dir = root.join(label);
            let updates_dir = case_dir.join("updates");
            std::fs::create_dir_all(&updates_dir).unwrap();

            let ran_marker = case_dir.join("ran.txt");
            let relaunch_marker = case_dir.join("relaunch.txt");
            let installer_exe = updates_dir.join("zoo2-updater-0.2.12-test.exe");
            let fake_exe = case_dir.join("zoo2-fake.exe");
            let lock_path = updater_lock_path(&installer_exe);
            let part_path = updater_part_path(&installer_exe);

            let mut installer_cfg = installer_exe.as_os_str().to_os_string();
            installer_cfg.push(".cfg");
            let mut fake_cfg = fake_exe.as_os_str().to_os_string();
            fake_cfg.push(".cfg");

            std::fs::copy(&stub_exe, &part_path).unwrap();
            std::fs::copy(&stub_exe, &fake_exe).unwrap();
            std::fs::write(installer_cfg, format!("{}\n{code}\n", ran_marker.display())).unwrap();
            std::fs::write(fake_cfg, format!("{}\n0\n", relaunch_marker.display())).unwrap();

            let (mut watcher, rx) = spawn_locked_powershell_watcher(
                INNO_WATCHER_SCRIPT,
                0,
                &installer_exe,
                &fake_exe,
                "/SILENT /DIR=\"C:\\zoo2\"",
                &lock_path,
            )
            .expect("watcher should lock cleanly");
            std::fs::rename(&part_path, &installer_exe).unwrap();
            await_watcher_signal(&mut watcher, &rx, "ARMED")
                .expect("watcher should arm after installer rename");

            let status = watcher.wait().unwrap();
            assert!(status.success(), "{label}: watcher exit status {status}");
            assert!(
                !installer_exe.exists(),
                "{label}: installer was not deleted"
            );
            assert!(!lock_path.exists(), "{label}: lock file was not deleted");

            let ran_args = std::fs::read_to_string(&ran_marker).unwrap();
            assert!(ran_args.contains("/SILENT"), "{label}: {ran_args}");
            assert!(
                ran_args.contains("/DIR=\"C:\\zoo2\""),
                "{label}: {ran_args}"
            );

            if should_relaunch {
                // Start-Process は非同期起動のため短い猶予を設けて sentinel を確認する。
                for _ in 0..40 {
                    if relaunch_marker.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let relaunch_args = std::fs::read_to_string(&relaunch_marker)
                    .unwrap_or_else(|e| panic!("{label}: missing relaunch marker: {e}"));
                assert!(
                    relaunch_args.contains(&format!("--update-failed {code}")),
                    "{label}: unexpected relaunch args {relaunch_args}"
                );
            } else {
                std::thread::sleep(Duration::from_millis(150));
                assert!(
                    !relaunch_marker.exists(),
                    "{label}: should not relaunch with --update-failed on exit 0"
                );
            }
        }

        // 作成〜監視保持中に期限切れ時刻で cleanup_old_inno_installers_in が走っても削除されないことを検証する。
        let race_dir = root.join("race-lock");
        std::fs::create_dir_all(&race_dir).unwrap();
        let active_installer = race_dir.join("zoo2-updater-0.2.12-active.exe");
        let active_part = updater_part_path(&active_installer);
        let active_lock = updater_lock_path(&active_installer);
        let ready_flag = race_dir.join("stop.flag");
        let hold_script = "\
$ErrorActionPreference = 'SilentlyContinue'\n\
$lockStream = [System.IO.File]::Open($env:ZOO2_UPDATER_LOCK, [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)\n\
[Console]::Out.WriteLine('LOCKED')\n\
[Console]::Out.Flush()\n\
while (-not (Test-Path -LiteralPath $env:ZOO2_UPDATER_ARGS)) { Start-Sleep -Milliseconds 50 }\n\
$lockStream.Dispose()\n\
Remove-Item -LiteralPath $env:ZOO2_UPDATER_LOCK -Force -ErrorAction SilentlyContinue\n";

        std::fs::write(&active_part, b"dummy").unwrap();
        let (mut holder, _rx) = spawn_locked_powershell_watcher(
            hold_script,
            0,
            &active_installer,
            &active_installer,
            &ready_flag.to_string_lossy(),
            &active_lock,
        )
        .unwrap();
        std::fs::rename(&active_part, &active_installer).unwrap();

        let future = std::time::SystemTime::now() + Duration::from_secs(3600);
        cleanup_old_inno_installers_in(&race_dir, future);
        assert!(
            active_installer.exists(),
            "cleanup must not delete locked installer even past grace age"
        );

        std::fs::write(&ready_flag, b"go").unwrap();
        assert!(holder.wait().unwrap().success());
        cleanup_old_inno_installers_in(&race_dir, future);
        assert!(
            !active_installer.exists(),
            "cleanup should remove stale unlocked installer after watcher releases lock"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inno_installer_args_quote_dir_without_trailing_backslash() {
        let dir = std::path::Path::new(r"C:\Program Files\zoo2\");
        let args = inno_installer_args(dir, 1234);
        assert!(args.contains(r#"/DIR="C:\Program Files\zoo2""#), "{args}");
        assert!(args.contains("/WAITPID=1234"), "{args}");
        assert!(args.contains("/RELAUNCH=1"), "{args}");
        assert!(args.contains("/SILENT"), "{args}");
    }

    #[test]
    fn should_keep_existing_version_prefers_newer_ready_update() {
        assert!(should_keep_existing_version("0.2.14", "0.2.13"));
        assert!(should_keep_existing_version("0.2.13", "0.2.13"));
        assert!(should_keep_existing_version("v0.2.14", "0.2.13"));
        assert!(!should_keep_existing_version("0.2.13", "0.2.14"));
        assert!(!should_keep_existing_version("0.2.14-rc.1", "0.2.14"));
        assert!(should_keep_existing_version("0.2.14-rc.2", "0.2.14-rc.1"));
    }

    #[test]
    fn encode_utf16le_base64_matches_powershell_format() {
        assert_eq!(encode_utf16le_base64("hi"), "aABpAA==");
        assert_eq!(
            encode_utf16le_base64("Write-Output 'hi'"),
            "VwByAGkAdABlAC0ATwB1AHQAcAB1AHQAIAAnAGgAaQAnAA=="
        );
    }

    #[test]
    fn watcher_script_reads_env_vars_and_waits_only_for_installer_process() {
        assert!(INNO_WATCHER_SCRIPT.contains("$env:ZOO2_UPDATER_INSTALLER"));
        assert!(INNO_WATCHER_SCRIPT.contains("$env:ZOO2_UPDATER_ARGS"));
        assert!(INNO_WATCHER_SCRIPT.contains("$env:ZOO2_UPDATER_LOCK"));
        assert!(INNO_WATCHER_SCRIPT.contains("$p.WaitForExit()"));
        assert!(!INNO_WATCHER_SCRIPT.contains(" -Wait"));
        assert!(INNO_WATCHER_SCRIPT.contains("--update-failed"));
    }

    #[test]
    fn stale_updater_artifact_check_respects_lock_and_grace_period() {
        let now = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(3600);
        let recent = now - Duration::from_secs(60);
        let stale = now - Duration::from_secs(20 * 60);
        assert!(!is_stale_updater_artifact(Some(recent), now, false));
        assert!(!is_stale_updater_artifact(Some(stale), now, true));
        assert!(!is_stale_updater_artifact(None, now, false));
        assert!(is_stale_updater_artifact(Some(stale), now, false));
        assert_eq!(
            updater_lock_path(std::path::Path::new(r"C:\updates\zoo2-updater-0.2.12.exe")),
            std::path::Path::new(r"C:\updates\zoo2-updater-0.2.12.exe.lock")
        );
    }

    #[test]
    fn take_inno_failure_message_parses_exit_codes() {
        assert_eq!(take_inno_failure_message(["--other", "1"]), None);
        assert_eq!(
            take_inno_failure_message(["--update-failed", "5"]).as_deref(),
            Some("インストーラーが終了しました (コード 5)")
        );
        assert_eq!(
            take_inno_failure_message(["--update-failed", "-1"]).as_deref(),
            Some("インストーラーを起動または実行できませんでした")
        );
        assert_eq!(
            take_inno_failure_message(["--update-failed"]).as_deref(),
            Some("インストーラーを起動または実行できませんでした")
        );
    }
}
