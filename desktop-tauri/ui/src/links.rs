//! 本文のアプリ内リンクの短い表示と埋め込み（docs/mfm.md の「チャンネル・メッセージのリンクの
//! 表示」）。状態の判定は `client-types`、開いているチャンネル以外のリンク先の取得は Rust 側が
//! 行う。ここは、画面の近くのリンク先を伝えて、分かったことを描き、押されたら移動する。

use std::collections::BTreeSet;

use jiff::civil::DateTime;
use leptos::{ev, html, prelude::*, task::spawn_local};
use wasm_bindgen::JsCast;
use zoo2_client_types::{
    ipc::{LinkedMessage, WatchLinks, command},
    link_store,
    links::{self, LinkState, LinkTarget, PreviewBody},
    rows::Message,
    shape::ShapeId,
    store::Store,
};

use crate::{Ctx, records, tauri};

/// 移動先のメッセージを、そのチャンネルの行が届くまで覚えておく。
#[derive(Debug, Clone, PartialEq)]
pub struct Jump {
    pub channel_id: String,
    pub message_id: String,
}

const REPLY_NOTICE: &str = "返信はスレッドの中にあります。この画面ではスレッドを開けないので、親のメッセージを表示しています。";
const UNAVAILABLE_MESSAGE: &str = "メッセージを表示できません";
const UNAVAILABLE_CHANNEL: &str = "チャンネルを表示できません";
/// 移動先の行を強調する長さ（`style.css` のアニメーションと同じ）。
const HIGHLIGHT: std::time::Duration = std::time::Duration::from_millis(2500);

/// 埋め込みに出す内容。
#[derive(Debug, Clone, PartialEq)]
pub enum Card {
    Loading,
    /// 見られない、または取得できなかった。押しても移動しない。
    Note(&'static str),
    Channel {
        name: String,
        note: Option<&'static str>,
        description: String,
    },
    Message {
        author: String,
        created_at: DateTime,
        place: String,
        /// 本文の要約か、本文を出さない理由 (`true`)。
        body: Option<(String, bool)>,
    },
}

/// URL の文字そのものを書いたリンクの代わりに出す短い表示。
#[derive(Debug, Clone, PartialEq)]
pub struct Short {
    /// メッセージへのリンク（吹き出しのアイコンを添える）。
    pub message: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// 行き先が分かったときだけ。
    pub inline: Option<Short>,
    pub card: Card,
}

fn display(state: &LinkState<'_>, store: &Store) -> Display {
    let only = |card| Display { inline: None, card };
    let channel_name = |channel: &zoo2_client_types::rows::Channel| {
        if channel.is_dm {
            "ダイレクトメッセージ".to_owned()
        } else {
            format!("#{}", channel.channel_name)
        }
    };
    match state {
        LinkState::Loading => only(Card::Loading),
        LinkState::Failed => only(Card::Note("リンク先を読み込めませんでした")),
        // 理由は区別せず、チャンネル名と本文を出さない。
        LinkState::Unavailable { message } => only(Card::Note(if *message {
            UNAVAILABLE_MESSAGE
        } else {
            UNAVAILABLE_CHANNEL
        })),
        LinkState::Channel { channel, encrypted } => {
            let name = channel_name(channel);
            let note = if *encrypted {
                Some("暗号化チャンネル")
            } else {
                (channel.is_private && !channel.is_dm).then_some("非公開チャンネル")
            };
            Display {
                inline: Some(Short {
                    message: false,
                    text: name.clone(),
                }),
                card: Card::Channel {
                    name,
                    note,
                    description: channel
                        .description
                        .as_deref()
                        .map(links::excerpt)
                        .unwrap_or_default(),
                },
            }
        }
        LinkState::Message { channel, preview } => {
            let author = store.display_name(&preview.author_id);
            let name = channel_name(channel);
            let place = if preview.parent_id.is_some() {
                format!("{name} 内のスレッド")
            } else {
                name.clone()
            };
            let body = match &preview.body {
                PreviewBody::Text(text) => Some((text.clone(), false)),
                PreviewBody::Encrypted => Some(("このメッセージは暗号化されています".into(), true)),
                PreviewBody::App => Some(("Appのメッセージ".into(), true)),
                PreviewBody::Forwarded => Some(("メッセージを転送しました".into(), true)),
                PreviewBody::Attachments(count) => Some((format!("添付ファイル {count} 件"), true)),
                PreviewBody::Empty => None,
            };
            Display {
                inline: Some(Short {
                    message: true,
                    text: format!("{author}さん: {name}"),
                }),
                card: Card::Message {
                    author,
                    created_at: preview.created_at,
                    place,
                    body,
                },
            }
        }
    }
}

/// 本文（`content`）を出すメッセージなら、その本文。部品が本文を隠すメッセージは `None`。
pub fn shown_content(message: &Message) -> Option<&str> {
    let hides = message
        .components_document()
        .is_some_and(|document| document.hides_content());
    (!hides).then_some(message.content.as_str())
}

impl Ctx {
    /// 接続先の起点。同じ origin の `/channels/<ID>` を、このアプリのリンクとして扱う。
    pub fn api_base_url(&self) -> String {
        self.session
            .with(|session| session.as_ref().map(|info| info.api_base_url.clone()))
            .unwrap_or_default()
    }

    /// 行き先の状態を `use_state` に渡す。読んだ値が変わると、呼び出し元は描き直される。
    fn with_link_state<R>(
        &self,
        target: &LinkTarget,
        use_state: impl FnOnce(&LinkState<'_>, &Store) -> R,
    ) -> R {
        let channel = self.channel.get();
        let open = link_store::Open {
            channel_id: channel.as_deref(),
            has_ciphertext: self.open_ciphertext.get(),
        };
        let recorded = records::recorded(&target.channel_id);
        self.store.with(|store| {
            self.linked.with(|fetched| {
                let state = link_store::state(store, fetched, open, recorded, target);
                use_state(&state, store)
            })
        })
    }

    pub fn link_display(&self, target: &LinkTarget) -> Display {
        self.with_link_state(target, display)
    }

    /// アプリ内リンクを開く。見られない行き先は開かず、理由を区別せずに知らせる。
    pub fn follow_link(&self, target: &LinkTarget) {
        let note = untrack(|| {
            self.with_link_state(target, |state, _| match state {
                LinkState::Unavailable { message: true } => Some(UNAVAILABLE_MESSAGE),
                LinkState::Unavailable { message: false } => Some(UNAVAILABLE_CHANNEL),
                _ => None,
            })
        });
        if let Some(note) = note {
            self.notice.set(Some(note));
            return;
        }
        // チャンネルの一覧の同期がまだなら、行き先が分からないので何もしない。
        if self
            .store
            .with_untracked(|store| store.channel(&target.channel_id).is_none())
        {
            return;
        }
        self.open_channel(target.channel_id.clone());
        self.notice.set(None);
        self.jump
            .set(target.linked_message_id().map(|message_id| Jump {
                channel_id: target.channel_id.clone(),
                message_id: message_id.to_owned(),
            }));
    }

    /// 本文のアプリ内リンクのクリック。WebView を移動させず、アプリの中で開く（修飾キーを
    /// 押していても同じ。同梱の画面には、別のタブがない）。
    pub fn click_link(&self, event: &ev::MouseEvent, target: &LinkTarget) {
        event.prevent_default();
        self.follow_link(target);
    }

    /// 画面の近くのリンク先を Rust 側に伝える。届く順番が入れ替わらないよう、前の呼び出しが
    /// 終わってから次を送る。
    fn watch_links(self, targets: Vec<LinkedMessage>) {
        if self
            .watched
            .with_value(|watched| watched.as_ref() == Some(&targets))
        {
            return;
        }
        self.watched.set_value(Some(targets));
        if self.watching.get_value() {
            return;
        }
        self.watching.set_value(true);
        let generation = self.generation.get_untracked();
        spawn_local(async move {
            let mut sent = None;
            while self.generation.get_untracked() == generation {
                let Some(targets) = self.watched.get_value() else {
                    break;
                };
                if sent.as_ref() == Some(&targets) {
                    break;
                }
                let links = WatchLinks {
                    targets: targets.clone(),
                };
                if let Err(err) =
                    tauri::invoke::<()>(command::WATCH_LINKS, serde_json::json!({ "links": links }))
                        .await
                {
                    if self.generation.get_untracked() == generation {
                        // 次に画面が変わったとき、同じ内容でも送り直す。
                        self.watched.set_value(None);
                        self.error.set(Some(err.message));
                    }
                    break;
                }
                sent = Some(targets);
            }
            self.watching.set_value(false);
        });
    }
}

/// 本文中の、メッセージへのリンクの短い表示に添える吹き出し。
#[component]
pub fn MessageIcon() -> impl IntoView {
    view! {
        <svg
            class="link-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
    }
}

/// 投稿の下の埋め込み。
#[component]
pub fn LinkCard(url: String, target: LinkTarget) -> impl IntoView {
    let ctx = expect_context::<Ctx>();
    let card = {
        let target = target.clone();
        Memo::new(move |_| ctx.link_display(&target).card)
    };
    move || {
        let open = {
            let target = target.clone();
            move |event: ev::MouseEvent| ctx.click_link(&event, &target)
        };
        match card.get() {
            Card::Loading => {
                view! { <div class="link-card note">"リンク先を読み込み中…"</div> }.into_any()
            }
            Card::Note(text) => view! { <div class="link-card note">{text}</div> }.into_any(),
            Card::Channel {
                name,
                note,
                description,
            } => view! {
                <a class="link-card" href=url.clone() on:click=open>
                    <span class="title">{name}</span>
                    {note.map(|note| view! { <span class="sub">{note}</span> })}
                    {(!description.is_empty())
                        .then(|| view! { <span class="excerpt short sub">{description}</span> })}
                </a>
            }
            .into_any(),
            Card::Message {
                author,
                created_at,
                place,
                body,
            } => view! {
                <a class="link-card" href=url.clone() on:click=open>
                    <span class="meta">
                        <span class="title">{author}</span>
                        <time>{card_time(created_at)}</time>
                    </span>
                    <span class="sub">{place}</span>
                    {body
                        .map(|(text, note)| {
                            view! { <span class="excerpt" class:sub=note>{text}</span> }
                        })}
                </a>
            }
            .into_any(),
        }
    }
}

/// タイムラインの時刻。今年でなければ年も付ける。
fn card_time(created_at: DateTime) -> String {
    let (year, time) = crate::local_time(created_at);
    if year == Some(js_sys::Date::new_0().get_full_year()) {
        time
    } else {
        year.map(|year| format!("{year}/{time}"))
            .unwrap_or_default()
    }
}

/// 一番下を見ているとみなす、残りの高さ。
const BOTTOM_SLACK: i32 = 40;

fn at_bottom(list: &web_sys::HtmlElement) -> bool {
    list.scroll_height() - list.scroll_top() - list.client_height() < BOTTOM_SLACK
}

/// 属性を持つ要素の一覧。
fn marked(list: &web_sys::Element, selector: &str) -> Vec<web_sys::Element> {
    let Ok(nodes) = list.query_selector_all(selector) else {
        return Vec::new();
    };
    (0..nodes.length())
        .filter_map(|index| nodes.item(index)?.dyn_into().ok())
        .collect()
}

/// 画面の近く（見える範囲と、前後の半画面）にある、リンクを持つメッセージの ID。
fn near_messages(list: &web_sys::Element) -> BTreeSet<String> {
    let view = list.get_bounding_client_rect();
    let margin = view.height() / 2.0;
    marked(list, "[data-links]")
        .into_iter()
        .filter(|element| {
            let rect = element.get_bounding_client_rect();
            rect.bottom() >= view.top() - margin && rect.top() <= view.bottom() + margin
        })
        .filter_map(|element| element.get_attribute("data-id"))
        .collect()
}

/// そのメッセージの行まで送り、少しのあいだ強調する。
fn show_message(list: &web_sys::Element, message_id: &str) {
    let Some(element) = marked(list, "[data-id]")
        .into_iter()
        .find(|element| element.get_attribute("data-id").as_deref() == Some(message_id))
    else {
        return;
    };
    let options = web_sys::ScrollIntoViewOptions::new();
    options.set_block(web_sys::ScrollLogicalPosition::Center);
    element.scroll_into_view_with_scroll_into_view_options(&options);
    let classes = element.class_list();
    let _ = classes.remove_1("jumped");
    // 外してすぐ付けても最初から再生するよう、一度レイアウトを計算させる。
    element.get_bounding_client_rect();
    let _ = classes.add_1("jumped");
    set_timeout(
        move || {
            let _ = classes.remove_1("jumped");
        },
        HIGHLIGHT,
    );
}

/// タイムラインのスクロール。新しい行が届いたら末尾へ、リンクで開いた行があればそこへ送り、
/// 画面の近くのリンク先を取得する。返すのは、スクロールのイベントで呼ぶ関数。
pub fn follow_timeline(
    channel_id: String,
    list: NodeRef<html::Div>,
    messages: Signal<Vec<(Message, String)>>,
) -> impl Fn() + Copy {
    let ctx = expect_context::<Ctx>();
    // 一番下を見ているか。リンク先が届いて行が伸びても、下を見ていれば下に留める。
    let stick = StoredValue::new(true);
    let near = RwSignal::new(BTreeSet::<String>::new());
    // 描画のあとで、画面の近くのメッセージを調べる。チャンネルを切り替えたあとに呼ばれても、
    // 破棄した値には触れない。
    let measure = move || {
        request_animation_frame(move || {
            let Some(Some(element)) = list.try_get_untracked() else {
                return;
            };
            let found = near_messages(&element);
            if near.try_with_untracked(|near| near != &found) == Some(true) {
                near.set(found);
            }
        });
    };
    let to_bottom = move || {
        // Effect の時点では新しい行がまだ描画されていないので、次の描画のときに
        // 伸びた後の `scrollHeight` を読んで末尾へ送る。
        request_animation_frame(move || {
            if let Some(Some(element)) = list.try_get_untracked() {
                element.set_scroll_top(element.scroll_height());
            }
        });
    };

    // 新しい行が届いたときだけ一番下まで送る。ほかの Shape の更新でもこの Effect は
    // 走るので、件数が増えていなければスクロールの位置を変えない。
    {
        let channel_id = channel_id.clone();
        Effect::new(move |previous: Option<usize>| {
            let count = messages.with(Vec::len);
            let jump = ctx.jump.get().filter(|jump| jump.channel_id == channel_id);
            if let Some(jump) = jump {
                // 行がそろうまで待つ。それまでは末尾へ送らない。
                let found = ctx.store.with(|store| {
                    store
                        .is_synced(&ShapeId::Messages(channel_id.clone()))
                        .then(|| {
                            store.message(&channel_id, &jump.message_id).map(|message| {
                                match &message.parent_id {
                                    Some(parent) => (parent.clone(), true),
                                    None => (message.id.clone(), false),
                                }
                            })
                        })
                });
                if let Some(found) = found {
                    ctx.jump.set(None);
                    match found {
                        Some((anchor, reply)) => {
                            stick.set_value(false);
                            ctx.notice.set(reply.then_some(REPLY_NOTICE));
                            request_animation_frame(move || {
                                if let Some(Some(element)) = list.try_get_untracked() {
                                    show_message(&element, &anchor);
                                }
                            });
                        }
                        None => ctx.notice.set(Some(UNAVAILABLE_MESSAGE)),
                    }
                    measure();
                }
                return count;
            }
            if previous.is_none_or(|previous| count > previous) {
                to_bottom();
            }
            measure();
            count
        });
    }

    Effect::new(move |_| {
        ctx.linked.track();
        if stick.get_value() {
            to_bottom();
        }
        measure();
    });

    Effect::new(move |_| {
        let ids = near.get();
        let base = ctx.api_base_url();
        let targets = ctx.store.with(|store| {
            let contents = ids
                .iter()
                .filter_map(|id| store.message(&channel_id, id))
                .filter_map(shown_content);
            link_store::wanted(store, Some(&channel_id), contents, &base)
        });
        ctx.watch_links(targets);
    });

    let resize = window_event_listener(ev::resize, move |_| measure());
    on_cleanup(move || resize.remove());

    move || {
        if let Some(Some(element)) = list.try_get_untracked() {
            stick.set_value(at_bottom(&element));
        }
        measure();
    }
}
