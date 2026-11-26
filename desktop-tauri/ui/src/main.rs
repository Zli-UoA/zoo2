//! (a) 同梱の Leptos の画面。ログイン、チャンネル一覧、タイムライン（本文のチャンネル・メッセージの
//! リンクの埋め込みを含む）、入力欄だけを持つ。
//!
//! 行は Rust 側から `zoo://shape` で届き、ここでは `Store` の写しに適用するだけ。起動や
//! 読み込み直しの後は `zoo_resync` で全体を送り直してもらう (`reset` 付きなので何度でもよい)。

mod components;
mod links;
mod records;
mod tauri;

use leptos::{ev, html, prelude::*, task::spawn_local};
use zoo2_client_types::{
    ipc::{
        FocusState, IpcError, Linked, LinkedMessage, LoginFailed, LoginPrompt, OpenChannel,
        OpenTarget, SendMessage, SendResult, SessionInfo, SessionState, ShapeBatch, ShapeFailed,
        UiMode, command, event,
    },
    link_store,
    links::{bare_links, embedded_links, parse_channel_link},
    mfm::{self, Tone},
    rows::Message,
    shape::ShapeId,
    store::Store,
};

use crate::links::{LinkCard, MessageIcon};

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
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
    /// 本文のリンクが指すメッセージで、Rust 側が取った結果。
    linked: RwSignal<link_store::Fetched>,
    /// 開いているチャンネルの同期した行に、暗号文の行があるか。
    open_ciphertext: Memo<bool>,
    /// Rust 側に伝えるリンク先と、伝えている途中かどうか。
    watched: StoredValue<Option<Vec<LinkedMessage>>>,
    watching: StoredValue<bool>,
    /// リンクで開いたメッセージ（その行が届くまで）。
    jump: RwSignal<Option<links::Jump>>,
    /// リンクで開けなかったときなどの知らせ。
    notice: RwSignal<Option<&'static str>>,
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
            self.linked.set(Default::default());
            self.watched.set_value(None);
            self.jump.set(None);
            self.notice.set(None);
        }
    }

    /// チャンネルを切り替える。前のチャンネルの行は捨ててから購読を切り替えてもらう。
    fn open_channel(&self, channel_id: String) {
        self.jump.set(None);
        self.notice.set(None);
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
    let store = RwSignal::new_local(Store::default());
    let channel = RwSignal::new(None::<String>);
    let open_ciphertext = Memo::new(move |_| {
        channel.get().is_some_and(|channel: String| {
            store.with(|store| link_store::has_ciphertext(store, &channel))
        })
    });
    let ctx = Ctx {
        session: RwSignal::new(None),
        store,
        channel,
        open_ciphertext,
        error: RwSignal::new(None),
        profile_request: RwSignal::new(None),
        profile_checked: RwSignal::new(0.0),
        generation: RwSignal::new(0),
        starting: RwSignal::new(false),
        linked: RwSignal::new(Default::default()),
        watched: StoredValue::new(None),
        watching: StoredValue::new(false),
        jump: RwSignal::new(None),
        notice: RwSignal::new(None),
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
            let channels = batch.shape == ShapeId::Channels;
            ctx.store
                .update(|store| store.apply(&batch.shape, batch.batch));
            // 画面を描き直す前に、暗号化チャンネルを記録する。
            if channels {
                ctx.store.with_untracked(|store| {
                    records::remember(
                        store
                            .channels()
                            .into_iter()
                            .filter(|channel| channel.encrypted)
                            .map(|channel| channel.id.as_str()),
                    )
                });
            }
        })
        .await;
        tauri::listen(event::LINKED, move |updates: Vec<Linked>| {
            ctx.linked.update(|linked| {
                linked.extend(
                    updates
                        .into_iter()
                        .map(|update| (update.target, update.fetch)),
                )
            });
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
            </nav>
            <main class="channel">
                <Notice />
                {move || match ctx.channel.get() {
                    Some(channel_id) => view! {
                        <ChannelHeading channel_id=channel_id.clone() />
                        <Timeline channel_id=channel_id.clone() />
                        <Footer channel_id=channel_id />
                    }
                    .into_any(),
                    None => view! { <p class="empty">"チャンネルを選んでください"</p> }.into_any(),
                }}
            </main>
        </div>
    }
}

/// 一覧にないチャンネル（未参加の公開チャンネルや、アーカイブしたもの）をリンクで開いたときの名前。
#[component]
fn ChannelHeading(channel_id: String) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let label = move || {
        let me = ctx.me().map(|me| me.id).unwrap_or_default();
        ctx.store.with(|store| {
            store
                .channel(&channel_id)
                .filter(|channel| {
                    channel.archive_at.is_some() || !store.is_member(&channel.id, &me)
                })
                .map(|channel| store.channel_label(channel, &me))
        })
    };
    move || label().map(|label| view! { <header class="channel-head">{label}</header> })
}

/// 知らせ。読み上げに伝わるよう、領域は常に置いて中身を入れ替える。
#[component]
fn Notice() -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let notice = move || {
        ctx.notice.get().map(|text| {
            view! {
                <button type="button" class="notice" title="閉じる" on:click=move |_| ctx.notice.set(None)>
                    <span>{text}</span>
                    <span aria-hidden="true">"×"</span>
                </button>
            }
        })
    };
    view! { <div role="status">{notice}</div> }
}

/// 入力欄。リンクで開いたアーカイブ済みのチャンネルには投稿できないので、理由を出す。
#[component]
fn Footer(channel_id: String) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let archived = {
        let channel_id = channel_id.clone();
        // 値が変わったときだけ作り直し、入力欄の書きかけを保つ。
        Memo::new(move |_| {
            ctx.store.with(|store| {
                store
                    .channel(&channel_id)
                    .is_some_and(|channel| channel.archive_at.is_some())
            })
        })
    };
    move || {
        if archived.get() {
            view! { <p class="composer archived">"アーカイブされたチャンネルには投稿できません"</p> }
                .into_any()
        } else {
            view! { <Composer channel_id=channel_id.clone() /> }.into_any()
        }
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
    let on_scroll = links::follow_timeline(channel_id, list, messages);
    view! {
        <div class="timeline" node_ref=list on:scroll=move |_| on_scroll()>
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

/// MFM の文字を行ごとに描画する。部品の文字 (`components.rs`) が使う。
fn mfm_view(content: String) -> AnyView {
    mfm_lines(content, false)
}

/// `shorten` なら、URL の文字そのものを書いたアプリ内リンクを短くする（メッセージの本文だけ）。
fn mfm_lines(content: String, shorten: bool) -> AnyView {
    let ctx = expect_context::<Ctx>();
    let me = ctx.me().map(|me| me.user_name);
    let lines = move || {
        let lines = ctx.store.with(|store| {
            mfm::parse_with_accounts(&content, me.as_deref(), &store.account_profiles)
        });
        let base = ctx.api_base_url();
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
                            Some(href) => match parse_channel_link(&href, &base) {
                                // このアプリのリンクは、WebView を移動させずにアプリの中で開く。
                                Some(target) => {
                                    let short = (shorten && span.content == href)
                                        .then(|| ctx.link_display(&target).inline)
                                        .flatten();
                                    let (icon, text) = match short {
                                        Some(short) => (short.message, short.text),
                                        None => (false, span.content),
                                    };
                                    view! {
                                        <a class=class href=href on:click=move |event| ctx.click_link(&event, &target)>
                                            {icon.then(|| view! { <MessageIcon /> })}
                                            {text}
                                        </a>
                                    }
                                    .into_any()
                                }
                                None => view! { <a class=class href=href>{span.content}</a> }.into_any(),
                            },
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
    let ctx = expect_context::<Ctx>();
    // メッセージ部品 (`components` 列。移行前の行は `card` 列から作る)。フラグがあれば本文を出さない
    let doc = message.components_document();
    let content = links::shown_content(&message).map(str::to_owned);
    let base = untrack(|| ctx.api_base_url());
    let text = content.as_deref().unwrap_or_default();
    let embeds = embedded_links(text, &base);
    // 画面の近くに来たときに、リンク先を取得する行。
    let has_links = !embeds.is_empty() || !bare_links(text, &base).is_empty();
    let body = content.map(|content| mfm_lines(content, true));
    let parts = doc.map(|doc| components::render(doc, &message));
    let cards = embeds
        .into_iter()
        .map(|(url, target)| view! { <LinkCard url=url target=target /> })
        .collect_view();
    let (_, time) = local_time(message.created_at);
    view! {
        <article class="message" data-id=message.id.clone() data-links=has_links.then_some("")>
            <header>
                <span class="author">{name}</span>
                <time>{time}</time>
            </header>
            {body}
            {parts}
            {cards}
        </article>
    }
}

/// UTC の作成時刻を、WebView のタイムゾーンの年と `MM/DD HH:MM` にする。夏時間で今と
/// オフセットが違う時期の投稿もずれないよう、投稿の時刻から JS の Date を作って変換する。
fn local_time(created_at: jiff::civil::DateTime) -> (Option<u32>, String) {
    let Ok(zoned) = created_at.to_zoned(jiff::tz::TimeZone::UTC) else {
        return (None, String::new());
    };
    let date = js_sys::Date::new(&(zoned.timestamp().as_millisecond() as f64).into());
    let time = format!(
        "{:02}/{:02} {:02}:{:02}",
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    );
    (Some(date.get_full_year()), time)
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
