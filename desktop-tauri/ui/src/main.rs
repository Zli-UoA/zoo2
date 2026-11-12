//! (a) 同梱の Leptos の画面。ログイン、チャンネル一覧、タイムライン、入力欄だけを持つ。
//!
//! 行は Rust 側から `zoo://shape` で届き、ここでは `Store` の写しに適用するだけ。起動や
//! 読み込み直しの後は `zoo_resync` で全体を送り直してもらう (`reset` 付きなので何度でもよい)。

mod tauri;

use leptos::{ev, html, prelude::*, task::spawn_local};
use zoo2_client_types::{
    ipc::{
        DeviceLogin, FocusState, IpcError, OpenChannel, OpenTarget, SendMessage, SendResult,
        SessionInfo, SessionState, ShapeBatch, ShapeFailed, UiMode, command, event,
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

#[derive(Clone, Copy)]
struct Ctx {
    session: RwSignal<Option<SessionInfo>>,
    store: RwSignal<Store, LocalStorage>,
    channel: RwSignal<Option<String>>,
    error: RwSignal<Option<String>>,
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
        self.session.set(Some(info));
        if ready && !was_ready && leptos {
            let ctx = *self;
            spawn_local(async move {
                ctx.report(tauri::invoke0::<()>(command::RESYNC).await);
            });
        }
        if !ready {
            self.store.set(Store::default());
            self.channel.set(None);
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

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx {
        session: RwSignal::new(None),
        store: RwSignal::new_local(Store::default()),
        channel: RwSignal::new(None),
        error: RwSignal::new(None),
    };
    provide_context(ctx);

    spawn_local(async move {
        // SHAPE を先に購読する。SESSION の Ready で呼ぶ `zoo_resync` の行を取りこぼさないため。
        tauri::listen(event::SHAPE, move |batch: ShapeBatch| {
            ctx.store
                .update(|store| store.apply(&batch.shape, batch.batch));
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
        {move || match ctx.session.get().map(|s| s.state) {
            Some(SessionState::Ready { .. }) => view! { <Main /> }.into_any(),
            state => view! { <Login state=state /> }.into_any(),
        }}
    }
}

#[component]
fn Login(state: Option<SessionState>) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let start = move |_| {
        spawn_local(async move {
            if let Err(err) = tauri::invoke0::<DeviceLogin>(command::START_LOGIN).await {
                ctx.error.set(Some(err.message));
            }
        });
    };
    let cancel = move |_| {
        spawn_local(async move {
            ctx.report(tauri::invoke0::<()>(command::CANCEL_LOGIN).await);
        });
    };
    let body = match state {
        None | Some(SessionState::Starting) => view! { <p>"読み込んでいます…"</p> }.into_any(),
        Some(SessionState::LoggedOut) => view! {
            <p>"zoo2 にログインしてください。"</p>
            <button on:click=start>"ブラウザーでログイン"</button>
        }
        .into_any(),
        Some(SessionState::LoggingIn { login }) => {
            let url = login
                .verification_uri_complete
                .clone()
                .unwrap_or(login.verification_uri.clone());
            view! {
                <p>"ブラウザーで次のページを開き、コードを確かめて承認してください。"</p>
                <p class="code">{login.user_code}</p>
                <p><a href=url.clone()>{url.clone()}</a></p>
                <p class="muted">"承認を待っています…"</p>
                <button on:click=cancel>"中止"</button>
            }
            .into_any()
        }
        Some(SessionState::Failed { message }) => view! {
            <p>"起動できませんでした。"</p>
            <pre>{message}</pre>
        }
        .into_any(),
        Some(SessionState::Ready { .. }) => ().into_any(),
    };
    view! { <div class="login">{body}</div> }
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
fn MessageView(message: Message, name: String) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let me = ctx.me().map(|me| me.user_name);
    let lines = mfm::parse(&message.content, me.as_deref());
    let lines = lines
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
        .collect_view();
    view! {
        <article class="message">
            <header>
                <span class="author">{name}</span>
                <time>{local_time(&message)}</time>
            </header>
            <div class="body">{lines}</div>
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
