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
            .is_some_and(|dir| is_inno_installed_in(dir) && has_matching_inno_registry(dir))
}

#[cfg(any(target_os = "windows", test))]
fn is_inno_installed_in(dir: &std::path::Path) -> bool {
    dir.join("unins000.exe").is_file()
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
        if matches_inno_uninstall_entry(
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
    raw.trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
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
fn matches_inno_uninstall_entry(
    exe_dir: &std::path::Path,
    install_location: Option<&str>,
    uninstall_string: Option<&str>,
) -> bool {
    let dir_norm = normalize_windows_path(&exe_dir.to_string_lossy());
    if dir_norm.is_empty() {
        return false;
    }
    let location_matches = install_location
        .map(normalize_windows_path)
        .is_some_and(|loc| loc == dir_norm);
    let expected_uninstaller =
        normalize_windows_path(&exe_dir.join("unins000.exe").to_string_lossy());
    let uninstaller_matches = uninstall_string
        .map(extract_exe_from_command)
        .map(normalize_windows_path)
        .is_some_and(|unins| unins == expected_uninstaller);
    location_matches && uninstaller_matches
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
    // deb では pkexec が、Windows ではインストーラー起動や UAC が待つことがあるため別スレッドで実行する。
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        if bundle_type().is_none() {
            match install_inno_setup(&app, &ready) {
                Ok(()) => app.exit(0),
                Err(err) => {
                    restore_ready_on_failure(&app, ready);
                    show_result(&app, "更新できませんでした", &err.to_string(), true);
                }
            }
            return;
        }
        let result = ready.update.install(&ready.bytes);
        match result {
            Ok(()) => app.restart(),
            Err(err) => {
                // パスワードの入力をやめたときなども、もう一度押せるように残す。
                // インストール中に新しい版の取得が終わっていた場合は、古い版で上書きしない。
                restore_ready_on_failure(&app, ready);
                show_result(&app, "更新できませんでした", &err.to_string(), true);
            }
        }
    });
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
fn updater_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("updates")
}

#[cfg(target_os = "windows")]
fn cleanup_old_inno_installers(app: &AppHandle) {
    let dir = updater_dir(app);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("zoo2-updater-")
            && name.ends_with(".exe")
            && std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn escape_ps_single_quoted(value: &str) -> Result<String, String> {
    if value.contains(['\r', '\n', '\0']) {
        return Err("パスに制御文字が含まれているため更新を起動できません".to_string());
    }
    Ok(value.replace('\'', "''"))
}

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

#[cfg(any(target_os = "windows", test))]
fn build_inno_watcher_script(
    old_pid: u32,
    installer_path: &std::path::Path,
    exe_path: &std::path::Path,
    args_line: &str,
) -> Result<String, String> {
    let installer = escape_ps_single_quoted(&installer_path.to_string_lossy())?;
    let exe = escape_ps_single_quoted(&exe_path.to_string_lossy())?;
    let args = escape_ps_single_quoted(args_line)?;
    Ok(format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         try {{\n\
           $old = Get-Process -Id {old_pid} -ErrorAction SilentlyContinue\n\
           if ($old -and $old.ProcessName -like 'zoo2*') {{ $old.WaitForExit(15000) | Out-Null }}\n\
         }} catch {{}}\n\
         $installer = '{installer}'\n\
         $exe = '{exe}'\n\
         $argsLine = '{args}'\n\
         $code = -1\n\
         try {{\n\
           $p = Start-Process -FilePath $installer -ArgumentList $argsLine -Wait -PassThru\n\
           if ($p) {{ $code = [int]$p.ExitCode }}\n\
         }} catch {{\n\
           $code = -1\n\
         }}\n\
         Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue\n\
         if ($code -ne 0 -and (Test-Path -LiteralPath $exe)) {{\n\
           Start-Process -FilePath $exe -ArgumentList @('--update-failed', [string]$code)\n\
         }}\n"
    ))
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
fn install_inno_setup(app: &AppHandle, ready: &Ready) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    ready
        .update
        .verify(&ready.bytes)
        .map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let install_dir = exe
        .parent()
        .ok_or_else(|| "インストール先ディレクトリが見つかりません".to_string())?;

    let dir = updater_dir(app);
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
    std::fs::write(&installer_path, &ready.bytes).map_err(|e| e.to_string())?;

    let args = inno_installer_args(install_dir, std::process::id());
    let script = match build_inno_watcher_script(std::process::id(), &installer_path, &exe, &args) {
        Ok(script) => script,
        Err(err) => {
            let _ = std::fs::remove_file(&installer_path);
            return Err(err);
        }
    };
    let encoded = encode_utf16le_base64(&script);

    // 親プロセス終了を待ってからインストーラーを待機実行し、非 0 終了時は失敗通知付きで再起動する。
    // {app} 配下のバイナリを監視役にすると上書きを妨げるため、一時実行をシステム PowerShell に委ねる。
    let spawned = std::process::Command::new(system_powershell_exe())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &encoded,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(err) = spawned {
        let _ = std::fs::remove_file(&installer_path);
        return Err(format!("インストーラーを起動できませんでした ({err})"));
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
    fn inno_installation_marker_detects_uninstaller() {
        let dir = std::env::temp_dir().join(format!("zoo2-inno-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_inno_installed_in(&dir));
        std::fs::write(dir.join("unins000.exe"), b"").unwrap();
        assert!(is_inno_installed_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inno_registry_entry_requires_matching_location_and_uninstaller() {
        let dir = std::path::Path::new(r"C:\Users\Alice\AppData\Local\Programs\zoo2");
        assert!(matches_inno_uninstall_entry(
            dir,
            Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
            Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\unins000.exe""#),
        ));
        assert!(matches_inno_uninstall_entry(
            dir,
            Some(r"c:/users/alice/appdata/local/programs/zoo2"),
            Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\UNINS000.EXE /SILENT"),
        ));
        assert!(!matches_inno_uninstall_entry(
            dir,
            Some(r"C:\Users\Bob\Programs\zoo2\"),
            Some(r#""C:\Users\Alice\AppData\Local\Programs\zoo2\unins000.exe""#),
        ));
        assert!(!matches_inno_uninstall_entry(
            dir,
            Some(r"C:\Users\Alice\AppData\Local\Programs\zoo2\"),
            None,
        ));
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
    fn watcher_script_escapes_single_quotes_and_rejects_newlines() {
        let installer = std::path::Path::new(r"C:\Users\O'Connor\updates\setup.exe");
        let exe = std::path::Path::new(r"C:\Users\O'Connor\zoo2\zoo2-desktop-tauri.exe");
        let script = build_inno_watcher_script(42, installer, exe, r#"/DIR="C:\O'Connor""#)
            .expect("valid paths");
        assert!(script.contains(r"C:\Users\O''Connor\updates\setup.exe"));
        assert!(script.contains("--update-failed"));

        let bad = std::path::Path::new("C:\\bad\npath\\setup.exe");
        assert!(build_inno_watcher_script(42, bad, exe, "/SILENT").is_err());
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
