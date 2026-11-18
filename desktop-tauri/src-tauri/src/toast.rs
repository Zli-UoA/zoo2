//! OS の通知の表示。
//!
//! `tauri-plugin-notification` はデスクトップでクリックを受け取れないため、Linux / macOS では
//! `notify-rust`、Windows では WinRT (`ToastNotificationManager`) を直接使う。
//! Linux (freedesktop) では action を付け、専用のスレッドで `wait_for_action` を待つ。
//! macOS では UNUserNotificationCenter で非同期に出すだけで、クリックには対応しない
//! (docs/desktop-tauri.md)。
//! Windows では AppUserModelID (`chat.zli.desktop`) を使ってトーストを出し、クリックや
//! 着信トーストの操作ボタン・終了時の自動非表示に対応する。

use std::sync::{Arc, Mutex, PoisonError};

use tauri::AppHandle;
use zoo2_client_types::ipc::OpenTarget;

/// Windows の AppUserModelID。Inno Setup のスタートメニューのショートカットと揃える。
#[cfg(target_os = "windows")]
pub const WINDOWS_APP_ID: &str = "chat.zli.desktop";

/// Windows で起動時に AppUserModelID (`HKCU\Software\Classes\AppUserModelId\chat.zli.desktop`) を登録する。
/// インストーラー経由でない起動 (`cargo tauri dev` や未インストール実行) でも、PowerShell ではなく
/// 「zoo2」の名前・アイコンで通知を出せるようにする。
#[cfg(target_os = "windows")]
pub fn ensure_windows_aumid(app: &AppHandle) {
    windows_impl::ensure_aumid(app);
}

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

/// 通知を出し、クリックされたら `on_click` を呼ぶ (Linux・Windows)。
pub fn show_with(
    app: &AppHandle,
    title: &str,
    body: &str,
    on_click: impl FnOnce(&AppHandle) + Send + 'static,
) {
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    #[cfg(unix)]
    let mut notification = {
        let mut notification = notify_rust::Notification::new();
        notification.appname("zoo2").summary(title).body(body);
        notification
    };

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

    #[cfg(target_os = "windows")]
    {
        windows_impl::show_notification(app, title, body, on_click);
    }
}

// ---- 通話の着信 (docs/calls.md) ----

/// 着信のトーストで押せる操作。`accept` と `decline` は Linux・Windows で使う（mac はクリックを受け取れない）。
pub struct CallActions {
    /// 「応答」。
    pub accept: Option<Box<dyn FnOnce() + Send>>,
    /// 「拒否」。
    pub decline: Option<Box<dyn FnOnce() + Send>>,
    /// 本文のクリック。ウィンドウを前に出すだけで、応答はしない。
    pub open: Box<dyn FnOnce() + Send>,
}

#[cfg(unix)]
#[derive(Clone)]
struct ToastHandle(Arc<notify_rust::NotificationHandle>);

#[cfg(target_os = "windows")]
#[derive(Clone)]
struct ToastHandle(String);

#[derive(Default)]
enum CallToastState {
    /// まだ出していない（別のスレッドで出している途中）。
    #[default]
    Pending,
    Shown(ToastHandle),
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
    #[cfg(unix)]
    fn shown(&self, handle: ToastHandle) -> Option<ToastHandle> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, CallToastState::Closed) {
            drop(state);
            close_handle(&handle);
            return None;
        }
        *state = CallToastState::Shown(handle.clone());
        Some(handle)
    }

    /// まだ閉じていないときだけ `show` を呼び、成功したら表示済みへ遷移させる。
    #[cfg(target_os = "windows")]
    fn show_if_open(&self, handle: ToastHandle, show: impl FnOnce() -> bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, CallToastState::Closed) {
            return false;
        }
        if show() {
            *state = CallToastState::Shown(handle);
            true
        } else {
            false
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn close_handle(handle: &ToastHandle) {
    tauri::async_runtime::block_on(handle.0.close_async());
}

#[cfg(target_os = "macos")]
fn close_handle(handle: &ToastHandle) {
    handle.0.close();
}

#[cfg(target_os = "windows")]
fn close_handle(handle: &ToastHandle) {
    windows_impl::close_toast(&handle.0);
}

/// ボタンを持てない環境（macOS）で、受け取った操作を使わずに捨てる。
/// フィールドを読むので、`-D warnings` の lint（未使用のフィールド）に掛からない。
#[cfg(target_os = "macos")]
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

    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    #[cfg(unix)]
    let mut notification = {
        let mut notification = notify_rust::Notification::new();
        notification.appname("zoo2").summary(title).body(body);
        notification
    };

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
            let Some(handle) = shared.shown(ToastHandle(Arc::new(handle))) else {
                return;
            };
            let CallActions {
                mut accept,
                mut decline,
                open,
            } = actions;
            let mut open = Some(open);
            tauri::async_runtime::block_on(handle.0.wait_for_action_async(move |response| {
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
                    let _ = shared.shown(ToastHandle(Arc::new(handle)));
                }
                Err(error) => eprintln!("通知を出せませんでした: {error}"),
            }
        });
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::show_call_invite(toast, title, body, actions);
    }
}

#[cfg(any(target_os = "windows", test))]
mod windows_xml {
    fn is_valid_xml_char(ch: char) -> bool {
        ch == '\t'
            || ch == '\n'
            || ch == '\r'
            || (ch >= '\u{0020}' && !matches!(ch, '\u{FFFE}' | '\u{FFFF}'))
    }

    pub(super) fn escape_xml(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c if is_valid_xml_char(c) => out.push(c),
                _ => out.push('\u{FFFD}'),
            }
        }
        out
    }

    pub(super) fn build_toast_xml(title: &str, body: &str) -> String {
        format!(
            r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
            escape_xml(title),
            escape_xml(body)
        )
    }

    pub(super) fn build_call_toast_xml(
        title: &str,
        body: &str,
        has_accept: bool,
        has_decline: bool,
    ) -> String {
        let mut buttons = String::new();
        if has_accept {
            buttons.push_str(
                r#"<action content="応答" arguments="accept" activationType="foreground"/>"#,
            );
        }
        if has_decline {
            buttons.push_str(
                r#"<action content="拒否" arguments="decline" activationType="foreground"/>"#,
            );
        }
        let actions = if buttons.is_empty() {
            String::new()
        } else {
            format!("<actions>{buttons}</actions>")
        };
        format!(
            r#"<toast scenario="reminder" duration="long"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>{actions}</toast>"#,
            escape_xml(title),
            escape_xml(body)
        )
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::{
        collections::HashMap,
        sync::{
            Arc, Mutex, OnceLock, PoisonError,
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
    };

    use tauri::{AppHandle, Manager};
    use windows::{
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{
            ToastActivatedEventArgs, ToastDismissedEventArgs, ToastFailedEventArgs,
            ToastNotification, ToastNotificationManager,
        },
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::{
                Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
                Registry::{
                    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
                    RegCloseKey, RegCreateKeyExW, RegSetValueExW,
                },
            },
        },
        core::{HSTRING, IInspectable, Interface, PCWSTR, h},
    };

    use super::{
        CallActions, CallToast, ToastHandle, WINDOWS_APP_ID,
        windows_xml::{build_call_toast_xml, build_toast_xml},
    };

    const CALL_TOAST_GROUP: &str = "calls";
    const APP_ICON_PNG: &[u8] = include_bytes!("../icons/128x128.png");
    const MAX_ACTIVE_TOASTS: usize = 64;
    static CALL_TOAST_SEQ: AtomicU64 = AtomicU64::new(1);
    static TOAST_INSTANCE_SEQ: AtomicU64 = AtomicU64::new(1);

    pub(super) struct ActiveToast {
        pub(super) tag: Option<String>,
        // WinRT のイベント通知を受け取る間、COM オブジェクトへの参照を保持する。
        _toast: ToastNotification,
    }

    static ACTIVE_TOASTS: OnceLock<Mutex<HashMap<u64, ActiveToast>>> = OnceLock::new();
    static TOAST_WORKER: OnceLock<mpsc::Sender<Box<dyn FnOnce() + Send>>> = OnceLock::new();

    pub(super) fn active_toasts() -> &'static Mutex<HashMap<u64, ActiveToast>> {
        ACTIVE_TOASTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(super) fn spawn_mta_task(task: impl FnOnce() + Send + 'static) {
        let tx = TOAST_WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
            std::thread::Builder::new()
                .name("winrt-toast".to_string())
                .spawn(move || {
                    // スレッドを終了させず MTA を維持し、表示後しばらく経ったクリックやボタン通知を受ける。
                    let _com = ComGuard::init();
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .expect("WinRT トーストスレッドの起動に失敗しました");
            tx
        });
        let _ = tx.send(Box::new(task));
    }

    pub(super) fn retain_toast(
        toast: &ToastNotification,
        tag: Option<&str>,
    ) -> windows::core::Result<u64> {
        let id = TOAST_INSTANCE_SEQ.fetch_add(1, Ordering::Relaxed);
        let dismissed =
            TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, _| {
                release_toast(id);
                Ok(())
            });
        toast.Dismissed(&dismissed)?;

        let failed =
            TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, _| {
                release_toast(id);
                Ok(())
            });
        toast.Failed(&failed)?;

        let mut map = active_toasts()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while map.len() >= MAX_ACTIVE_TOASTS {
            let Some(&oldest_id) = map.keys().min() else {
                break;
            };
            map.remove(&oldest_id);
        }
        map.insert(
            id,
            ActiveToast {
                tag: tag.map(ToOwned::to_owned),
                _toast: toast.clone(),
            },
        );
        Ok(id)
    }

    fn release_toast(id: u64) {
        active_toasts()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }

    pub(super) fn release_toast_by_tag(tag: &str) {
        active_toasts()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, entry| entry.tag.as_deref() != Some(tag));
    }

    /// COM アパートメントをマルチスレッドで初期化する。`Drop` で対になる終了処理を呼ぶ。
    struct ComGuard(bool);

    impl ComGuard {
        fn init() -> Self {
            // SAFETY: 現在のスレッドの COM アパートメントを初期化するだけ。
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            // RPC_E_CHANGED_MODE は呼び出し元スレッドがすでに別のアパートメントで初期化済み。
            // その場合は自前で CoUninitialize を呼ばない。
            let should_uninit = hr != RPC_E_CHANGED_MODE && hr.is_ok();
            Self(should_uninit)
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: ComGuard::init がこのスレッドで S_OK/S_FALSE を返した場合のみ呼ぶ。
                unsafe { CoUninitialize() };
            }
        }
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_reg_sz(hkey: HKEY, name: &str, value: &str) {
        let wide_name = to_wide(name);
        let wide_value = to_wide(value);
        // SAFETY: 文字列スライスのバイト長 (NUL 終端含む) を渡す。
        let bytes = unsafe {
            std::slice::from_raw_parts(
                wide_value.as_ptr().cast::<u8>(),
                wide_value.len() * std::mem::size_of::<u16>(),
            )
        };
        // SAFETY: 開いた HKEY に REG_SZ 値を書き込む。
        let _ =
            unsafe { RegSetValueExW(hkey, PCWSTR(wide_name.as_ptr()), None, REG_SZ, Some(bytes)) };
    }

    pub(super) fn ensure_aumid(app: &AppHandle) {
        let icon_path = app.path().app_local_data_dir().ok().and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("app-icon.png");
            let unchanged = std::fs::read(&path).is_ok_and(|existing| existing == APP_ICON_PNG);
            if !unchanged {
                std::fs::write(&path, APP_ICON_PNG).ok()?;
            }
            Some(path)
        });

        let subkey = to_wide(&format!(
            r"Software\Classes\AppUserModelId\{WINDOWS_APP_ID}"
        ));
        let mut hkey = HKEY::default();
        // SAFETY: HKCU 配下に自アプリ用の AppUserModelId キーを作成または開く。
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
        };
        if status.is_err() {
            return;
        }
        set_reg_sz(hkey, "DisplayName", "zoo2");
        if let Some(icon_path) = icon_path {
            set_reg_sz(hkey, "IconUri", &icon_path.to_string_lossy());
        }
        // SAFETY: RegCreateKeyExW で取得したハンドルを閉じる。
        let _ = unsafe { RegCloseKey(hkey) };
    }

    type ToastClickCallback = Arc<Mutex<Option<Box<dyn FnOnce(&AppHandle) + Send>>>>;

    pub(super) fn show_notification(
        app: &AppHandle,
        title: &str,
        body: &str,
        on_click: impl FnOnce(&AppHandle) + Send + 'static,
    ) {
        let app = app.clone();
        let xml = build_toast_xml(title, body);
        let on_click: ToastClickCallback = Arc::new(Mutex::new(Some(Box::new(on_click))));
        spawn_mta_task(move || {
            if let Err(err) = show_notification_inner(&app, &xml, on_click) {
                eprintln!("通知を出せませんでした: {err}");
            }
        });
    }

    fn show_notification_inner(
        app: &AppHandle,
        xml: &str,
        on_click: ToastClickCallback,
    ) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let toast_id = retain_toast(&toast, None)?;
        let app_clone = app.clone();
        let handler = TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
            release_toast(toast_id);
            if let Some(callback) = on_click
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                callback(&app_clone);
            }
            Ok(())
        });
        if let Err(err) = toast.Activated(&handler) {
            release_toast(toast_id);
            return Err(err);
        }
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(WINDOWS_APP_ID))?;
        if let Err(err) = notifier.Show(&toast) {
            release_toast(toast_id);
            return Err(err);
        }
        Ok(())
    }

    pub(super) fn show_call_invite(
        toast: &CallToast,
        title: &str,
        body: &str,
        actions: CallActions,
    ) {
        let shared = toast.clone();
        let xml = build_call_toast_xml(
            title,
            body,
            actions.accept.is_some(),
            actions.decline.is_some(),
        );
        let seq = CALL_TOAST_SEQ.fetch_add(1, Ordering::Relaxed);
        // WinRT の Tag は旧 Windows 10 で最大 16 文字の制限があるため短く保つ。
        let tag = format!("call-{seq:x}");
        spawn_mta_task(move || {
            if shared.is_closed() {
                return;
            }
            if let Err(err) = show_call_invite_inner(&shared, &tag, &xml, actions) {
                eprintln!("着信通知を出せませんでした: {err}");
            }
        });
    }

    pub(super) fn dispatch_call_action(actions: CallActions, arg: &str) {
        match arg {
            "accept" => {
                if let Some(accept) = actions.accept {
                    accept();
                }
            }
            "decline" => {
                if let Some(decline) = actions.decline {
                    decline();
                }
            }
            _ => (actions.open)(),
        }
    }

    fn show_call_invite_inner(
        shared: &CallToast,
        tag: &str,
        xml: &str,
        actions: CallActions,
    ) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetTag(&HSTRING::from(tag))?;
        toast.SetGroup(&HSTRING::from(CALL_TOAST_GROUP))?;
        let toast_id = retain_toast(&toast, Some(tag))?;

        let actions = Arc::new(Mutex::new(Some(actions)));
        let handler = TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
            release_toast(toast_id);
            let Some(actions) = actions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            else {
                return Ok(());
            };
            let arg = args
                .as_ref()
                .and_then(|insp| insp.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|a| a.Arguments().ok())
                .map(|s| s.to_string_lossy())
                .unwrap_or_default();
            dispatch_call_action(actions, arg.as_str());
            Ok(())
        });
        if let Err(err) = toast.Activated(&handler) {
            release_toast(toast_id);
            return Err(err);
        }

        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(WINDOWS_APP_ID))?;
        let mut show_err = None;
        let shown = shared.show_if_open(ToastHandle(tag.to_string()), || {
            match notifier.Show(&toast) {
                Ok(()) => true,
                Err(err) => {
                    show_err = Some(err);
                    false
                }
            }
        });
        if !shown {
            release_toast(toast_id);
            if let Some(err) = show_err {
                return Err(err);
            }
        }
        Ok(())
    }

    pub(super) fn close_toast(tag: &str) {
        release_toast_by_tag(tag);
        let tag = tag.to_string();
        spawn_mta_task(move || {
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.RemoveGroupedTagWithId(
                    &HSTRING::from(tag),
                    h!("calls"),
                    &HSTRING::from(WINDOWS_APP_ID),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::windows_xml::{build_call_toast_xml, build_toast_xml, escape_xml};

    #[test]
    fn windows_toast_xml_escapes_special_characters() {
        let xml = build_toast_xml("A & <B>", "\"hello\" 'world'\u{000B}next\t\n🎉\u{FFFE}");
        assert!(xml.contains("A &amp; &lt;B&gt;"));
        assert!(xml.contains("&quot;hello&quot; &apos;world&apos;\u{FFFD}next\t\n🎉\u{FFFD}"));
        assert!(!xml.contains('\u{000B}'));
        assert!(!xml.contains('\u{FFFE}'));
        assert_eq!(escape_xml("normal"), "normal");

        #[cfg(target_os = "windows")]
        {
            use windows::{Data::Xml::Dom::XmlDocument, core::HSTRING};
            let doc = XmlDocument::new().unwrap();
            doc.LoadXml(&HSTRING::from(&xml)).unwrap();
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn call_toast_skips_show_after_close() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let toast = super::CallToast::default();
        toast.close();
        let called = AtomicBool::new(false);
        let shown = toast.show_if_open(super::ToastHandle("call-1".into()), || {
            called.store(true, Ordering::SeqCst);
            true
        });
        assert!(!shown);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn windows_call_toast_xml_includes_expected_buttons() {
        let with_both = build_call_toast_xml("Caller", "着信", true, true);
        assert!(with_both.contains(r#"arguments="accept""#));
        assert!(with_both.contains(r#"arguments="decline""#));
        assert!(with_both.contains(r#"scenario="reminder""#));

        let decline_only = build_call_toast_xml("Caller", "着信", false, true);
        assert!(!decline_only.contains(r#"arguments="accept""#));
        assert!(decline_only.contains(r#"arguments="decline""#));

        #[cfg(target_os = "windows")]
        {
            use windows::{Data::Xml::Dom::XmlDocument, core::HSTRING};
            for sample in [with_both, decline_only] {
                let doc = XmlDocument::new().unwrap();
                doc.LoadXml(&HSTRING::from(&sample)).unwrap();
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_toast_dispatch_routes_accept_decline_and_body_click() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let run = |arg: &str| -> (usize, usize, usize) {
            let accepted = Arc::new(AtomicUsize::new(0));
            let declined = Arc::new(AtomicUsize::new(0));
            let opened = Arc::new(AtomicUsize::new(0));
            let a = accepted.clone();
            let d = declined.clone();
            let o = opened.clone();
            super::windows_impl::dispatch_call_action(
                super::CallActions {
                    accept: Some(Box::new(move || {
                        a.fetch_add(1, Ordering::SeqCst);
                    })),
                    decline: Some(Box::new(move || {
                        d.fetch_add(1, Ordering::SeqCst);
                    })),
                    open: Box::new(move || {
                        o.fetch_add(1, Ordering::SeqCst);
                    }),
                },
                arg,
            );
            (
                accepted.load(Ordering::SeqCst),
                declined.load(Ordering::SeqCst),
                opened.load(Ordering::SeqCst),
            )
        };

        assert_eq!(run("accept"), (1, 0, 0));
        assert_eq!(run("decline"), (0, 1, 0));
        assert_eq!(run(""), (0, 0, 1));
        assert_eq!(run("unknown"), (0, 0, 1));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_toast_worker_retains_winrt_com_objects_until_release() {
        use std::sync::mpsc;
        use windows::{
            Data::Xml::Dom::XmlDocument,
            Foundation::TypedEventHandler,
            UI::Notifications::ToastNotification,
            core::{HSTRING, IInspectable},
        };

        let xml = build_call_toast_xml("Test", "着信テスト", true, true);
        let (done_tx, done_rx) = mpsc::channel();
        super::windows_impl::spawn_mta_task(move || {
            let result = (|| -> windows::core::Result<()> {
                let doc = XmlDocument::new()?;
                doc.LoadXml(&HSTRING::from(&xml))?;
                let toast = ToastNotification::CreateToastNotification(&doc)?;
                let handler =
                    TypedEventHandler::<ToastNotification, IInspectable>::new(|_, _| Ok(()));
                toast.Activated(&handler)?;
                let tag = "test-retain-1";
                let id = super::windows_impl::retain_toast(&toast, Some(tag))?;
                {
                    let map = super::windows_impl::active_toasts()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    assert!(map.contains_key(&id));
                }
                super::windows_impl::release_toast_by_tag(tag);
                {
                    let map = super::windows_impl::active_toasts()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    assert!(!map.contains_key(&id));
                }
                Ok(())
            })();
            let _ = done_tx.send(result);
        });
        done_rx.recv().unwrap().unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires interactive Windows notification session"]
    fn windows_live_action_center_toast_roundtrip() {
        use std::{sync::mpsc, time::Duration};
        use windows::{
            Data::Xml::Dom::XmlDocument,
            Foundation::TypedEventHandler,
            UI::Notifications::{ToastNotification, ToastNotificationManager},
            core::{HSTRING, IInspectable},
        };

        let xml = build_call_toast_xml("zoo2-test", "WinRT toast verification", true, true);
        let tag = "smoke-call-1";
        let (done_tx, done_rx) = mpsc::channel();
        super::windows_impl::spawn_mta_task(move || {
            let result = (|| -> windows::core::Result<()> {
                let doc = XmlDocument::new()?;
                doc.LoadXml(&HSTRING::from(&xml))?;
                let toast = ToastNotification::CreateToastNotification(&doc)?;
                toast.SetTag(&HSTRING::from(tag))?;
                toast.SetGroup(&HSTRING::from("calls"))?;
                let _id = super::windows_impl::retain_toast(&toast, Some(tag))?;
                let handler =
                    TypedEventHandler::<ToastNotification, IInspectable>::new(|_, _| Ok(()));
                toast.Activated(&handler)?;
                let notifier = ToastNotificationManager::CreateToastNotifierWithId(
                    &HSTRING::from(super::WINDOWS_APP_ID),
                )?;
                notifier.Show(&toast)?;
                Ok(())
            })();
            let _ = done_tx.send(result);
        });
        done_rx.recv().unwrap().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        {
            let map = super::windows_impl::active_toasts()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(
                map.values().any(|t| t.tag.as_deref() == Some(tag)),
                "toast COM reference must stay retained after worker callback returns"
            );
        }
        super::windows_impl::close_toast(tag);
        std::thread::sleep(Duration::from_millis(300));
        let map = super::windows_impl::active_toasts()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!map.values().any(|t| t.tag.as_deref() == Some(tag)));
    }
}
