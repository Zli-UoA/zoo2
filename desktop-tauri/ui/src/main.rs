//! (a) 同梱の Leptos の画面。ログイン、チャンネル一覧、タイムライン、入力欄だけを持つ。
//!
//! 行は Rust 側から `zoo://shape` で届き、ここでは `Store` の写しに適用するだけ。起動や
//! 読み込み直しの後は `zoo_resync` で全体を送り直してもらう (`reset` 付きなので何度でもよい)。

mod components;
mod tauri;

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use leptos::{ev, html, prelude::*, task::spawn_local};
use zoo2_client_types::{
    api::NotificationLevel,
    ipc::{
        ChannelNotificationSettings, FocusState, IpcError, LoginFailed, LoginPrompt, OpenChannel,
        OpenTarget, SendMessage, SendResult, SessionInfo, SessionState,
        SetChannelNotificationLevel, ShapeBatch, ShapeFailed, UiMode, command, event,
    },
    mfm::{self, Tone},
    rows::Message,
    shape::ShapeId,
    store::Store,
};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// WebView は単一スレッドなので、TUI の `tokio::sync::Mutex` の代わりに待ち順にロックを渡す
/// 非同期 Mutex を使う。ロック待ちはタスクの中で行い、クリックやキー操作を待たせない。
#[derive(Clone, Default)]
struct AsyncMutex {
    inner: Rc<RefCell<MutexInner>>,
}

#[derive(Default)]
struct MutexInner {
    locked: bool,
    /// 解放したロックを渡す相手。新しい操作が割り込んで順番が入れ替わらないようにする。
    next_owner: Option<Waker>,
    waiters: VecDeque<Waker>,
}

impl AsyncMutex {
    async fn lock(&self) -> MutexGuard {
        poll_fn(|cx| {
            let mut inner = self.inner.borrow_mut();
            if !inner.locked {
                inner.locked = true;
                return Poll::Ready(MutexGuard {
                    inner: Rc::clone(&self.inner),
                });
            }
            if inner
                .next_owner
                .as_ref()
                .is_some_and(|owner| owner.will_wake(cx.waker()))
            {
                // 前の所有者から渡されたぶんは `locked` を true のまま受け取る。
                inner.next_owner = None;
                return Poll::Ready(MutexGuard {
                    inner: Rc::clone(&self.inner),
                });
            }
            if !inner
                .waiters
                .iter()
                .any(|waiter| waiter.will_wake(cx.waker()))
            {
                inner.waiters.push_back(cx.waker().clone());
            }
            Poll::Pending
        })
        .await
    }
}

struct MutexGuard {
    inner: Rc<RefCell<MutexInner>>,
}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        let next = {
            let mut inner = self.inner.borrow_mut();
            match inner.waiters.pop_front() {
                Some(waiter) => {
                    inner.next_owner = Some(waiter.clone());
                    Some(waiter)
                }
                None => {
                    inner.locked = false;
                    None
                }
            }
        };
        if let Some(waiter) = next {
            waiter.wake();
        }
    }
}

/// 通知設定の保存要求。開始時の世代と要求 ID を持たせ、アカウント切替や連続操作で
/// 古い完了が新しい表示を壊さないようにする。`in_flight` はこのチャンネルの未完了件数で、
/// 自世代の完了ごとに減算する（TUI の `notification_level_in_flight` と同じ方針）。
#[derive(Clone)]
struct NotificationSave {
    generation: u64,
    request: u64,
    level: NotificationLevel,
    in_flight: usize,
}

#[derive(Clone, Copy)]
struct Ctx {
    session: RwSignal<Option<SessionInfo>>,
    store: RwSignal<Store, LocalStorage>,
    channel: RwSignal<Option<String>>,
    error: RwSignal<Option<String>>,
    profile_request: RwSignal<Option<u64>>,
    profile_checked: RwSignal<f64>,
    generation: RwSignal<u64>,
    starting: RwSignal<bool>,
    /// 同じチャンネルへの PATCH を送信順に直列化するロック。コンポーネントが再生成されても
    /// 失われないよう、アプリ全体の状態に置く（TUI の `notification_level_locks` と同じ方針）。
    notification_locks: StoredValue<HashMap<String, AsyncMutex>, LocalStorage>,
    /// チャンネルごとに最後に選んだ値。Shape がこの値に追いつくまで表示と同値判定に使う。
    notification_saved: RwSignal<HashMap<String, NotificationLevel>, LocalStorage>,
    /// 保存中のチャンネル。別チャンネルに移っても保存中であることを保つ。
    /// 世代と要求 ID を持たせ、古い保存の完了が新しい保存の表示を上書きしないようにする。
    notification_pending: RwSignal<HashMap<String, NotificationSave>, LocalStorage>,
    /// 保存要求の連番。アカウントをまたいで単調に増やし、同じ値の連続選択も識別する。
    notification_request: RwSignal<u64>,
}

impl Ctx {
    fn me(&self) -> Option<zoo2_client_types::api::Me> {
        match self.session.get()?.state {
            SessionState::Ready { me } => Some(me),
            _ => None,
        }
    }

    fn report(&self, result: Result<(), IpcError>) {
        if let Err(err) = result {
            self.error.set(Some(err.message));
        }
    }

    fn set_session(&self, info: SessionInfo) {
        let ready = matches!(info.state, SessionState::Ready { .. });
        // remote では Ready になると Rust 側が WebView を zli.chat へ移すので、この画面の
        // 同期は要らない (`zoo_resync` も leptos 専用で WrongMode になる)。
        let leptos = info.ui == UiMode::Leptos;
        let was_ready = matches!(
            self.session.get_untracked().map(|s| s.state),
            Some(SessionState::Ready { .. })
        );
        if !matches!(info.state, SessionState::LoggedOut) {
            self.starting.set(false);
        }
        self.session.set(Some(info));
        if ready && !was_ready && leptos {
            let ctx = *self;
            spawn_local(async move {
                ctx.report(tauri::invoke0::<()>(command::RESYNC).await);
                refresh_account_profiles(ctx).await;
            });
        }
        if !ready {
            self.generation.update(|generation| *generation += 1);
            self.profile_checked.set(0.0);
            self.store.set(Store::default());
            self.channel.set(None);
            // 別のアカウントで同じチャンネルを開いても、前の選択値を表示しない。
            self.notification_saved.set(HashMap::new());
            self.notification_pending.set(HashMap::new());
        }
    }

    /// チャンネルを切り替える。前のチャンネルの行は捨ててから購読を切り替えてもらう。
    fn open_channel(&self, channel_id: String) {
        if let Some(old) = self.channel.get_untracked() {
            if old == channel_id {
                return;
            }
            self.store
                .update(|store| store.forget(&ShapeId::Messages(old)));
        }
        self.channel.set(Some(channel_id.clone()));
        let ctx = *self;
        spawn_local(async move {
            let open = OpenChannel {
                channel_id: channel_id.clone(),
            };
            ctx.report(
                tauri::invoke::<()>(command::OPEN_CHANNEL, serde_json::json!({ "open": open }))
                    .await,
            );
            send_focus(Some(channel_id)).await;
        });
    }
}

async fn send_focus(channel_id: Option<String>) {
    let focus = FocusState {
        focused: tauri::has_focus(),
        channel_id,
    };
    let _ = tauri::invoke::<()>(command::SET_FOCUS, serde_json::json!({ "focus": focus })).await;
}

async fn refresh_account_profiles(ctx: Ctx) {
    let generation = ctx.generation.get_untracked();
    if !tauri::has_focus()
        || ctx.me().is_none()
        || ctx.profile_request.get_untracked() == Some(generation)
        || js_sys::Date::now() - ctx.profile_checked.get_untracked() < 60_000.0
        || !matches!(
            ctx.session.get_untracked().map(|info| info.ui),
            Some(UiMode::Leptos)
        )
    {
        return;
    }
    ctx.profile_request.set(Some(generation));
    let profiles =
        tauri::invoke0::<zoo2_client_types::accounts::AccountProfiles>(command::ACCOUNT_PROFILES)
            .await
            .unwrap_or_default();
    if ctx.profile_request.get_untracked() == Some(generation) {
        ctx.profile_request.set(None);
    }
    if ctx.generation.get_untracked() == generation && ctx.me().is_some() {
        ctx.profile_checked.set(js_sys::Date::now());
        ctx.store.update(|store| store.account_profiles = profiles);
    }
}

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx {
        session: RwSignal::new(None),
        store: RwSignal::new_local(Store::default()),
        channel: RwSignal::new(None),
        error: RwSignal::new(None),
        profile_request: RwSignal::new(None),
        profile_checked: RwSignal::new(0.0),
        generation: RwSignal::new(0),
        starting: RwSignal::new(false),
        notification_locks: StoredValue::new_local(HashMap::new()),
        notification_saved: RwSignal::new_local(HashMap::new()),
        notification_pending: RwSignal::new_local(HashMap::new()),
        notification_request: RwSignal::new(0),
    };
    provide_context(ctx);
    if let Ok(timer) = set_interval_with_handle(
        move || {
            spawn_local(refresh_account_profiles(ctx));
        },
        std::time::Duration::from_secs(60),
    ) {
        on_cleanup(move || timer.clear());
    }

    spawn_local(async move {
        // SHAPE を先に購読する。SESSION の Ready で呼ぶ `zoo_resync` の行を取りこぼさないため。
        tauri::listen(event::SHAPE, move |batch: ShapeBatch| {
            let settings_changed = batch.shape == ShapeId::ChannelNotificationSettings;
            ctx.store
                .update(|store| store.apply(&batch.shape, batch.batch));
            if settings_changed {
                // Shape が選んだ値まで届いたら上書きをやめて store を正本に戻す。
                // 古い値のあいだは選んだ値を保ち、同じ値の再送を防ぐ (TUI と同じ)。
                let me = ctx
                    .session
                    .get_untracked()
                    .and_then(|info| match info.state {
                        SessionState::Ready { me } => Some(me.id),
                        _ => None,
                    });
                if let Some(me) = me {
                    ctx.store.with(|store| {
                        ctx.notification_saved.update(|saved| {
                            saved.retain(|channel_id, level| {
                                store.notification_level(channel_id, &me) != *level
                            });
                        });
                    });
                }
            }
        })
        .await;
        tauri::listen(event::SESSION, move |info: SessionInfo| {
            ctx.set_session(info)
        })
        .await;
        tauri::listen(event::SHAPE_FAILED, move |failed: ShapeFailed| {
            ctx.error.set(Some(failed.message));
        })
        .await;
        tauri::listen(event::LOGIN_FAILED, move |failed: LoginFailed| {
            ctx.error.set(Some(failed.message));
        })
        .await;
        tauri::listen(event::OPEN, move |target: OpenTarget| {
            ctx.open_channel(target.channel_id);
        })
        .await;
        match tauri::invoke0::<SessionInfo>(command::SESSION).await {
            Ok(info) => ctx.set_session(info),
            Err(err) => ctx.error.set(Some(err.message)),
        }
    });

    // 前面かどうかが変わったら、表示中のチャンネルと一緒に知らせる (通知を抑えるため)。
    let _ = window_event_listener(ev::focus, move |_| {
        spawn_local(refresh_account_profiles(ctx));
        spawn_local(send_focus(ctx.channel.get_untracked()));
    });
    let _ = window_event_listener(ev::blur, move |_| {
        spawn_local(send_focus(ctx.channel.get_untracked()));
    });

    view! {
        <Show when=move || ctx.error.get().is_some()>
            <div class="error" on:click=move |_| ctx.error.set(None)>
                {move || ctx.error.get().unwrap_or_default()}
            </div>
        </Show>
        {move || match ctx.session.get().map(|s| (s.ui, s.state)) {
            Some((UiMode::Leptos, SessionState::Ready { .. })) => view! { <Main /> }.into_any(),
            // (b) ではログインしたら Rust 側がウィンドウをリモートの画面へ移す。
            Some((UiMode::Remote, SessionState::Ready { .. })) => {
                view! { <Login state=Some(SessionState::Starting) /> }.into_any()
            }
            session => view! { <Login state=session.map(|(_, state)| state) /> }.into_any(),
        }}
    }
}

#[component]
fn ZooMark() -> impl IntoView {
    view! {
        <svg class="auth-logo" viewBox="0 0 256 256" aria-hidden="true">
            <g transform="translate(42.905 41.975) scale(.93)">
                <path
                    d="M91.0854 60C107.654 60 121.085 46.5685 121.085 30C121.085 13.4315 107.654 0 91.0854 0C74.5169 0 61.0854 13.4315 61.0854 30C61.0854 46.5685 74.5169 60 91.0854 60Z"
                    fill="currentColor"
                />
                <path
                    d="M152.171 82.2368C168.739 82.2368 182.171 68.8054 182.171 52.2368C182.171 35.6683 168.739 22.2368 152.171 22.2368C135.602 22.2368 122.171 35.6683 122.171 52.2368C122.171 68.8054 135.602 82.2368 152.171 82.2368Z"
                    fill="currentColor"
                />
                <path
                    d="M30 82.2368C46.5685 82.2368 60 68.8054 60 52.2368C60 35.6683 46.5685 22.2368 30 22.2368C13.4315 22.2368 0 35.6683 0 52.2368C0 68.8054 13.4315 82.2368 30 82.2368Z"
                    fill="currentColor"
                />
                <path
                    d="M91.0859 65C124.223 65.0003 151.086 91.8631 151.086 125C151.086 157.878 124.641 184.58 91.8613 184.995L91.0859 185H37.0859C33.7722 185 31.085 182.314 31.085 179V125C31.085 91.8629 57.9489 65 91.0859 65Z"
                    fill="currentColor"
                />
            </g>
        </svg>
    }
}

#[component]
fn ExternalLinkIcon() -> impl IntoView {
    view! {
        <svg
            class="auth-btn-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="M15 3h6v6" />
            <path d="M10 14 21 3" />
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
        </svg>
    }
}

#[component]
fn Login(state: Option<SessionState>) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let starting = ctx.starting;
    // 既定はブラウザー（認可コード + PKCE）。ブラウザーを開けない環境のために、確認コード
    // （デバイス認可）でも始められる。
    let begin = move |method: &'static str| {
        if starting.get_untracked() {
            return;
        }
        starting.set(true);
        ctx.error.set(None);
        spawn_local(async move {
            if let Err(err) = tauri::invoke::<LoginPrompt>(
                command::START_LOGIN,
                serde_json::json!({ "method": method }),
            )
            .await
            {
                ctx.error.set(Some(err.message));
            }
            starting.set(false);
        });
    };
    let start = move |_| begin("browser");
    let start_device = move |_| begin("device");
    let cancel = move |_| {
        spawn_local(async move {
            ctx.report(tauri::invoke0::<()>(command::CANCEL_LOGIN).await);
        });
    };
    let panel = match state {
        None | Some(SessionState::Starting) => view! {
            <div class="auth-eyebrow">
                <span class="auth-pill" aria-hidden="true"></span>
                <span>"ZOO2 DESKTOP"</span>
            </div>
            <h1 class="auth-title">"読み込んでいます"</h1>
            <p class="auth-lead">"セッションを確認しています…"</p>
            <div class="auth-status">
                <span class="auth-dot" aria-hidden="true"></span>
                <span>"しばらくお待ちください"</span>
            </div>
        }
        .into_any(),
        Some(SessionState::LoggedOut) => view! {
            <div class="auth-eyebrow">
                <span class="auth-pill" aria-hidden="true"></span>
                <span>"WELCOME BACK"</span>
            </div>
            <h1 class="auth-title">"ログイン"</h1>
            <p class="auth-lead">"zoo2 を始めるには、ブラウザーで Zli アカウントを認証してください。"</p>
            <div class="auth-actions">
                <button
                    type="button"
                    class="auth-btn auth-btn-primary"
                    disabled=move || starting.get()
                    on:click=start
                >
                    <ExternalLinkIcon />
                    <span>
                        {move || if starting.get() { "ブラウザーを開いています…" } else { "ブラウザーでログイン" }}
                    </span>
                </button>
            </div>
            <p class="auth-hint">"ボタンを押すと、通常は既定のブラウザーが自動で開きます。開かない場合は、次の画面から手動で開けます。"</p>
            <div class="auth-fallback">
                <p class="auth-fallback-note">"ブラウザーが開けない環境では、確認コードでもログインできます。"</p>
                <button
                    type="button"
                    class="auth-btn auth-btn-outline"
                    disabled=move || starting.get()
                    on:click=start_device
                >
                    "確認コードでログイン"
                </button>
            </div>
        }
        .into_any(),
        Some(SessionState::LoggingIn {
            login: LoginPrompt::Browser(login),
        }) => view! {
            <div class="auth-eyebrow">
                <span class="auth-pill" aria-hidden="true"></span>
                <span>"BROWSER LOGIN"</span>
            </div>
            <h1 class="auth-title">"ブラウザーで承認"</h1>
            <p class="auth-lead">"開いたブラウザーでログインを済ませると、このアプリに戻ります。"</p>
            <div class="auth-status">
                <span class="auth-dot" aria-hidden="true"></span>
                <span>"承認を待っています…"</span>
            </div>
            <div class="auth-actions">
                <a
                    class="auth-btn auth-btn-primary"
                    href=login.url.clone()
                    target="_blank"
                    rel="noopener noreferrer"
                >
                    <ExternalLinkIcon />
                    <span>"ブラウザーで開く"</span>
                </a>
                <button type="button" class="auth-btn auth-btn-outline" on:click=cancel>
                    "中止"
                </button>
            </div>
            <div class="auth-fallback">
                <p class="auth-fallback-note">"自動で開かない場合は上のボタンを押してください。ブラウザーが開けない環境では、確認コードでもログインできます。"</p>
                <button
                    type="button"
                    class="auth-btn auth-btn-outline"
                    disabled=move || starting.get()
                    on:click=start_device
                >
                    "確認コードでログイン"
                </button>
            </div>
        }
        .into_any(),
        Some(SessionState::LoggingIn {
            login: LoginPrompt::Device(login),
        }) => {
            let has_complete = login.verification_uri_complete.is_some();
            let lead = if !login.auto_open {
                "下の「ブラウザーで開く」を押し、表示された画面に確認コードを入力して承認してください。"
            } else if has_complete {
                "開いたブラウザーの画面に表示されているコードが一致しているか確かめて承認してください。"
            } else {
                "開いたブラウザーの画面に下の確認コードを入力して承認してください。"
            };
            let url = login
                .verification_uri_complete
                .clone()
                .unwrap_or(login.verification_uri.clone());
            view! {
                <div class="auth-eyebrow">
                    <span class="auth-pill" aria-hidden="true"></span>
                    <span>"DEVICE AUTHORIZATION"</span>
                </div>
                <h1 class="auth-title">"ブラウザーで承認"</h1>
                <p class="auth-lead">{lead}</p>
                <div class="auth-code-card">
                    <span class="auth-code-label">"確認コード"</span>
                    <p class="auth-code">{login.user_code}</p>
                </div>
                <div class="auth-status">
                    <span class="auth-dot" aria-hidden="true"></span>
                    <span>"承認を待っています…"</span>
                </div>
                <div class="auth-actions">
                    <a
                        class="auth-btn auth-btn-primary"
                        href=url.clone()
                        target="_blank"
                        rel="noopener noreferrer"
                    >
                        <ExternalLinkIcon />
                        <span>"ブラウザーで開く"</span>
                    </a>
                    <button type="button" class="auth-btn auth-btn-outline" on:click=cancel>
                        "中止"
                    </button>
                </div>
                <div class="auth-fallback">
                    <p class="auth-fallback-note">"自動で開かない場合は上のボタンまたは下のリンクを押してください。"</p>
                    <a class="auth-url" href=url.clone() target="_blank" rel="noopener noreferrer">{url.clone()}</a>
                </div>
            }
            .into_any()
        }
        Some(SessionState::Failed { message }) => view! {
            <div class="auth-eyebrow">
                <span class="auth-pill" aria-hidden="true"></span>
                <span>"STARTUP ERROR"</span>
            </div>
            <h1 class="auth-title">"起動できませんでした"</h1>
            <pre class="auth-failed">{message}</pre>
        }
        .into_any(),
        Some(SessionState::Ready { .. }) => ().into_any(),
    };
    view! {
        <div class="auth-shell">
            <aside class="auth-hero" aria-hidden="true">
                <div class="auth-hero-inner">
                    <ZooMark />
                </div>
            </aside>
            <main class="auth-main">
                <section class="auth-card">{panel}</section>
            </main>
        </div>
    }
}

#[component]
fn Main() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let logout = move |_| {
        spawn_local(async move {
            ctx.report(tauri::invoke0::<()>(command::LOGOUT).await);
        });
    };
    view! {
        <div class="layout">
            <nav class="sidebar">
                <header>
                    <span>{move || ctx.me().map(|me| me.display_name).unwrap_or_default()}</span>
                    <button class="link" on:click=logout>"ログアウト"</button>
                </header>
                <ChannelList />
                <ChannelNotificationSettings />
            </nav>
            <main class="channel">
                {move || match ctx.channel.get() {
                    Some(channel_id) => view! { <Timeline channel_id=channel_id.clone() /> <Composer channel_id=channel_id /> }.into_any(),
                    None => view! { <p class="empty">"チャンネルを選んでください"</p> }.into_any(),
                }}
            </main>
        </div>
    }
}

#[component]
fn ChannelList() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let channels = move || {
        let me = ctx.me().map(|me| me.id).unwrap_or_default();
        ctx.store.with(|store| {
            store
                .channels()
                .into_iter()
                .filter(|channel| channel.archive_at.is_none() && store.is_member(&channel.id, &me))
                .map(|channel| (channel.id.clone(), store.channel_label(channel, &me)))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <ul class="channels">
            <For each=channels key=|(id, label)| (id.clone(), label.clone()) let:item>
                {
                    let (id, label) = item;
                    let selected = {
                        let id = id.clone();
                        move || ctx.channel.get().as_deref() == Some(id.as_str())
                    };
                    let current = selected.clone();
                    view! {
                        <li>
                            <button
                                class:selected=selected
                                aria-current=move || current().then_some("page")
                                on:click=move |_| ctx.open_channel(id.clone())
                            >
                                {label}
                            </button>
                        </li>
                    }
                }
            </For>
        </ul>
    }
}

/// 選択中のチャンネルの通知レベル。現在値と 3 段階の選択を出し、変更は IPC で保存する。
///
/// 現在値と同値判定は本人用 Shape (`channel_notification_settings`) の行から読む。Android 等で
/// 変えた値も `ctx.store` に届くので、コマンドの応答を待たずに表示が更新される。行が届くまでは
/// 読み込み中として選択を止める（ストア既定の `mentions` を現在値として扱うと、実際が `mute` の
/// ときに同値判定で PATCH が送られない）。
///
/// 保存した値は Shape が追いつくまで保ち、表示と同値判定に使う。Shape が届く前に別の値を選んでも
/// 古い値との同値判定で PATCH が省略されないようにするため。同じチャンネルへの PATCH は
/// アプリ全体のロックで送信順に直列化し、別チャンネルは並列で送る（TUI と同じ方針）。
/// 各保存は開始時の世代と要求 ID を持ち、完了時にそれが最新のときだけ表示を更新するので、
/// アカウント切替後に古い保存が完了しても新しい選択を上書きしない。
#[component]
fn ChannelNotificationSettings() -> impl IntoView {
    let ctx = expect_context::<Ctx>();

    // 選択値と同値判定は Shape の設定行と保存済み値から読む。行がまだ届いていなければ `None` なので、
    // 読み込み中として全選択肢を止める。
    let setting = Signal::derive(move || {
        let channel_id = ctx.channel.get()?;
        let me = ctx.me()?;
        ctx.store.with(|store| {
            store.notification_setting(&channel_id, &me.id).map(|_| {
                ctx.notification_saved
                    .with(|saved| saved.get(&channel_id).copied())
                    .unwrap_or_else(|| store.notification_level(&channel_id, &me.id))
            })
        })
    });

    let saving = Signal::derive(move || {
        let Some(channel_id) = ctx.channel.get() else {
            return false;
        };
        ctx.notification_pending
            .with(|pending| pending.contains_key(&channel_id))
    });

    let save = move |next: NotificationLevel| {
        let Some(channel_id) = ctx.channel.get_untracked() else {
            return;
        };
        // 同値判定は Shape が古くても最後に選んだ値に対して行う。行が無いときは選択自体が無効。
        let Some(previous) = setting.get_untracked() else {
            return;
        };
        if previous == next {
            return;
        }
        // クリックを待たせず、タスクの中でチャンネルごとのロックを取る (Mutex は待ち順が公平)。
        let mut lock = None;
        ctx.notification_locks.update_value(|locks| {
            lock = Some(locks.entry(channel_id.clone()).or_default().clone());
        });
        let lock = lock.expect("通知設定のロックを取得できませんでした");
        // 開始時の世代と要求 ID を保存する。アカウント切替後に古い完了が届いても、
        // 新しい保存の表示を上書きしないようにするため (TUI の世代判定と同じ方針)。
        let generation = ctx.generation.get_untracked();
        let request_id = ctx.notification_request.get_untracked() + 1;
        ctx.notification_request.set(request_id);
        ctx.notification_saved.update(|saved| {
            saved.insert(channel_id.clone(), next);
        });
        ctx.notification_pending.update(|pending| {
            let in_flight = pending
                .get(&channel_id)
                .filter(|save| save.generation == generation)
                .map_or(1, |save| save.in_flight + 1);
            pending.insert(
                channel_id.clone(),
                NotificationSave {
                    generation,
                    request: request_id,
                    level: next,
                    in_flight,
                },
            );
        });
        spawn_local(async move {
            let _guard = lock.lock().await;
            let request = SetChannelNotificationLevel {
                channel_id: channel_id.clone(),
                level: next,
            };
            let result = tauri::invoke::<ChannelNotificationSettings>(
                command::SET_CHANNEL_NOTIFICATION_LEVEL,
                serde_json::json!({ "request": request }),
            )
            .await;
            let succeeded = result.is_ok();
            // 未完了件数は自世代の完了ごとに減算する。ロック待ちの要求が残っていれば 0 に
            // ならないので、最後の完了まで保存中を保てる。アカウント切替後の要求は
            // 古い完了では減算しない。
            let mut latest = false;
            let mut finished = false;
            if ctx.notification_pending.with(|pending| {
                pending
                    .get(&channel_id)
                    .is_some_and(|save| save.generation == generation)
            }) {
                ctx.notification_pending.update(|pending| {
                    if let Some(save) = pending.get_mut(&channel_id) {
                        if save.generation == generation {
                            save.in_flight = save.in_flight.saturating_sub(1);
                            latest = save.request == request_id && save.level == next;
                            finished = save.in_flight == 0;
                        }
                    }
                });
            }
            if finished {
                ctx.notification_pending.update(|pending| {
                    pending.remove(&channel_id);
                });
            }
            // 失敗した値が最新の選択のときだけ、書き込み前の値に戻す。
            if !succeeded
                && latest
                && ctx
                    .notification_saved
                    .with(|saved| saved.get(&channel_id) == Some(&next))
            {
                ctx.notification_saved.update(|saved| {
                    saved.insert(channel_id.clone(), previous);
                });
            }
            if latest {
                if let Err(err) = result {
                    ctx.error.set(Some(err.message));
                }
            }
            // 成功した値は Shape が届くまで保持し、届いたら App の SHAPE 処理が解除する。
        });
    };

    view! {
        <Show when=move || ctx.channel.get().is_some()>
            <section class="notification-settings">
                <div class="b">"通知"</div>
                <Show when=move || setting.get().is_none()>
                    <p class="notification-help" role="status">"通知設定を読み込み中…"</p>
                </Show>
                <div class="row">
                    {move || {
                        NotificationLevel::ALL
                            .into_iter()
                            .map(|candidate| {
                                let (label, help) = match candidate {
                                    NotificationLevel::All => (
                                        "すべてのメッセージ",
                                        "通常の投稿も通知します。暗号化チャンネルでも選べます。",
                                    ),
                                    NotificationLevel::Mentions => (
                                        "メンションと返信（既定）",
                                        "あなたへのメンション、返信、DM、リアクションを通知します。",
                                    ),
                                    NotificationLevel::Mute => (
                                        "ミュート",
                                        "通常の通知は届きません。明示的なメンションは届きます。",
                                    ),
                                };
                                let selected = move || setting.get() == Some(candidate);
                                view! {
                                    <button
                                        class:selected=selected
                                        disabled=move || saving.get() || setting.get().is_none()
                                        on:click=move |_| save(candidate)
                                    >
                                        <span class="label">{label}</span>
                                        <small>{help}</small>
                                    </button>
                                }
                                .into_any()
                            })
                            .collect_view()
                    }}
                </div>
            </section>
        </Show>
    }
}

#[component]
fn Timeline(channel_id: String) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let messages = {
        let channel_id = channel_id.clone();
        Signal::derive(move || {
            ctx.store.with(|store| {
                store
                    .root_messages(&channel_id)
                    .into_iter()
                    .map(|message| (message.clone(), store.display_name(&message.user_id)))
                    .collect::<Vec<_>>()
            })
        })
    };
    let list = NodeRef::<html::Div>::new();
    // 新しい行が届いたときだけ一番下まで送る。ほかの Shape の更新でもこの Effect は
    // 走るので、件数が増えていなければスクロールの位置を変えない。
    Effect::new(move |previous: Option<usize>| {
        let count = messages.with(Vec::len);
        if previous.is_none_or(|previous| count > previous) {
            // Effect の時点では新しい行がまだ描画されていないので、次の描画のときに
            // 伸びた後の `scrollHeight` を読んで末尾へ送る。
            request_animation_frame(move || {
                if let Some(list) = list.get_untracked() {
                    list.set_scroll_top(list.scroll_height());
                }
            });
        }
        count
    });
    view! {
        <div class="timeline" node_ref=list>
            <For
                each=move || messages.get()
                key=|(message, name)| (message.id.clone(), message.updated_at, name.clone())
                let:item
            >
                <MessageView message=item.0 name=item.1 />
            </For>
        </div>
    }
}

#[component]
fn AccountIcon(url: Option<String>, initial: String, label: String) -> impl IntoView {
    let failed = RwSignal::new(false);
    let url = url.filter(|value| value.starts_with("https://") || value.starts_with("http://"));
    view! {
        <span role="img" aria-label=label style="display:inline-flex;width:1.15em;height:1.15em;vertical-align:middle;align-items:center;justify-content:center;border-radius:50%">
            {move || {
                if !failed.get() && let Some(url) = &url {
                    return view! { <img src=url.clone() alt="" style="width:100%;height:100%;border-radius:50%" on:error=move |_| failed.set(true) /> }.into_any();
                }
                initial.clone().into_any()
            }}
        </span>
    }
}

/// MFM の本文を行ごとに描画する。部品の文字 (`components.rs`) も同じ描画を使う。
fn mfm_view(content: String) -> AnyView {
    let ctx = expect_context::<Ctx>();
    let me = ctx.me().map(|me| me.user_name);
    let lines = move || {
        let lines = ctx.store.with(|store| {
            mfm::parse_with_accounts(&content, me.as_deref(), &store.account_profiles)
        });
        lines
        .into_iter()
        .map(|line| {
            let spans = line
                .spans
                .into_iter()
                .map(|span| {
                    let style = span.style;
                    let mut class = String::new();
                    for (on, name) in [
                        (style.bold, "b"),
                        (style.italic, "i"),
                        (style.crossed_out, "s"),
                        (style.underlined, "u"),
                        (style.dim, "dim"),
                        (style.reversed, "rev"),
                    ] {
                        if on {
                            class.push_str(name);
                            class.push(' ');
                        }
                    }
                    if let Some(initial) = span.avatar_fallback {
                        return view! { <AccountIcon url=span.avatar_url initial=initial label=span.content /> }.into_any();
                    }
                    match style.tone {
                        Tone::Link => match span.href {
                            Some(href) => {
                                view! { <a class=class href=href>{span.content}</a> }.into_any()
                            }
                            None => view! { <span class=class>{span.content}</span> }.into_any(),
                        },
                        tone => {
                            class.push_str(match tone {
                                Tone::Code => "code",
                                Tone::Mention => "mention",
                                Tone::Hashtag => "hashtag",
                                Tone::Math => "math",
                                Tone::Quote => "quote",
                                _ => "",
                            });
                            view! { <span class=class>{span.content}</span> }.into_any()
                        }
                    }
                })
                .collect_view();
            view! { <div class="line">{spans}</div> }
        })
        .collect_view()
    };
    view! { <div class="body">{lines}</div> }.into_any()
}

#[component]
fn MessageView(message: Message, name: String) -> impl IntoView {
    // メッセージ部品 (`components` 列。移行前の行は `card` 列から作る)。フラグがあれば本文を出さない
    let doc = message.components_document();
    let hides = doc.as_ref().is_some_and(|d| d.hides_content());
    let body = (!hides).then(|| mfm_view(message.content.clone()));
    let parts = doc.map(|doc| components::render(doc, &message));
    view! {
        <article class="message">
            <header>
                <span class="author">{name}</span>
                <time>{local_time(&message)}</time>
            </header>
            {body}
            {parts}
        </article>
    }
}

/// UTC の作成時刻を、WebView のタイムゾーンの `MM/DD HH:MM` にする。夏時間で今と
/// オフセットが違う時期の投稿もずれないよう、投稿の時刻から JS の Date を作って変換する。
fn local_time(message: &Message) -> String {
    let Ok(zoned) = message.created_at.to_zoned(jiff::tz::TimeZone::UTC) else {
        return String::new();
    };
    let date = js_sys::Date::new(&(zoned.timestamp().as_millisecond() as f64).into());
    format!(
        "{:02}/{:02} {:02}:{:02}",
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

/// 入力欄。日本語の変換中 (`isComposing`) の Enter では送らない。値は DOM が持ち、
/// 変換中に書き戻さない (候補ウィンドウの位置や確定前の文字を壊さないため)。
#[component]
fn Composer(channel_id: String) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let input = NodeRef::<html::Textarea>::new();
    let sending = RwSignal::new(false);
    let send = move |channel_id: String| {
        let Some(textarea) = input.get() else {
            return;
        };
        let content = textarea.value();
        if content.trim().is_empty() || sending.get_untracked() {
            return;
        }
        sending.set(true);
        spawn_local(async move {
            let sent = content.clone();
            let message = SendMessage {
                id: uuid::Uuid::new_v4().to_string(),
                channel_id,
                content,
                parent_id: None,
            };
            match tauri::invoke::<SendResult>(
                command::SEND_MESSAGE,
                serde_json::json!({ "message": message }),
            )
            .await
            {
                // 送信を待つ間に書き足された分 (変換中の文字も含む) は消さない。
                Ok(_) if textarea.value() == sent => textarea.set_value(""),
                Ok(_) => {}
                Err(err) => ctx.error.set(Some(err.message)),
            }
            sending.set(false);
        });
    };
    let on_keydown = {
        let channel_id = channel_id.clone();
        move |ev: ev::KeyboardEvent| {
            // keyCode 229 は、isComposing を立てない古い IME の変換中の Enter。
            #[allow(deprecated)]
            let composing = ev.is_composing() || ev.key_code() == 229;
            if ev.key() == "Enter" && !ev.shift_key() && !composing {
                ev.prevent_default();
                send(channel_id.clone());
            }
        }
    };
    view! {
        <div class="composer">
            <textarea
                node_ref=input
                rows="2"
                placeholder="メッセージを送信 (Shift+Enter で改行)"
                on:keydown=on_keydown
            ></textarea>
        </div>
    }
}
