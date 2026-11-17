//! メッセージ部品（Discord Components V2 形式、frontend の `ComponentTree.tsx`）。
//!
//! - 文字は MFM で描画する（`mfm_view`）。
//! - 画像と動画は取得せず、説明とリンクで出す（WebView から置き場所へ直接取りに行かないため）。
//! - ボタンと選択肢は押すと `zoo_invoke_app_action` でアプリへ送る。入力欄やピッカーは Web 版で操作する。

use leptos::{prelude::*, task::spawn_local};
use zoo2_client_types::{
    components::{
        Component, Confirm, Document, SelectOption, button_style, validate::is_https_url,
    },
    ipc::{InvokeAppAction, command},
    rows::Message,
};

use crate::{Ctx, mfm_view, tauri};

#[derive(Clone)]
struct Target {
    app_id: Option<String>,
    channel_id: String,
    message_id: String,
}

pub fn render(doc: Document, message: &Message) -> AnyView {
    let target = Target {
        app_id: message.app_origin_id.clone(),
        channel_id: message.channel_id.clone(),
        message_id: message.id.clone(),
    };
    let nodes = doc
        .components
        .into_iter()
        .map(|c| node(c, &target))
        .collect_view();
    view! { <div class="components">{nodes}</div> }.into_any()
}

fn send(target: &Target, custom_id: String, value: serde_json::Value) {
    let Some(app_id) = target.app_id.clone() else {
        return;
    };
    let ctx = expect_context::<Ctx>();
    let action = InvokeAppAction {
        app_id,
        channel_id: target.channel_id.clone(),
        message_id: target.message_id.clone(),
        custom_id,
        value,
    };
    spawn_local(async move {
        if let Err(err) = tauri::invoke::<()>(
            command::INVOKE_APP_ACTION,
            serde_json::json!({ "action": action }),
        )
        .await
        {
            ctx.error.set(Some(err.message));
        }
    });
}

fn link(label: String, url: &str) -> AnyView {
    if is_https_url(url) {
        view! { <a href=url.to_string() target="_blank" rel="noopener noreferrer">{label}</a> }
            .into_any()
    } else {
        view! { <span>{label}</span> }.into_any()
    }
}

fn small(text: String) -> AnyView {
    view! { <div class="small">{text}</div> }.into_any()
}

fn web_only() -> AnyView {
    small("この操作は Web 版で行えます".into())
}

/// 押すとアプリへ送るボタン。confirm があれば、まずその場に確認（はい・いいえ）を出し、はいで送る。
fn action_button(
    class: &'static str,
    label: String,
    off: bool,
    confirm: Option<Confirm>,
    target: Target,
    custom_id: String,
    value: serde_json::Value,
) -> AnyView {
    let pending = RwSignal::new(false);
    let has_confirm = confirm.is_some();
    let (t2, c2, v2) = (target.clone(), custom_id.clone(), value.clone());
    view! {
        <span class="confirmable">
            <button class=class disabled=off on:click=move |_| {
                if has_confirm { pending.set(true) } else { send(&target, custom_id.clone(), value.clone()) }
            }>
                {label}
            </button>
            {move || {
                let confirm = confirm.clone()?;
                pending.get().then(|| {
                    let (t3, c3, v3) = (t2.clone(), c2.clone(), v2.clone());
                    let class = if confirm.style.as_deref() == Some("danger") { "button danger" } else { "button primary" };
                    view! {
                        <span class="confirm">
                            <span class="b">{confirm.title.clone()}</span>
                            <span class="small">{confirm.text.clone()}</span>
                            <button class=class on:click=move |_| { pending.set(false); send(&t3, c3.clone(), v3.clone()) }>
                                {confirm.confirm.clone()}
                            </button>
                            <button class="button" on:click=move |_| pending.set(false)>{confirm.deny.clone()}</button>
                        </span>
                    }
                })
            }}
        </span>
    }
    .into_any()
}

fn choices(
    target: &Target,
    custom_id: String,
    options: Vec<SelectOption>,
    disabled: bool,
    array: bool,
    confirm: Option<Confirm>,
) -> AnyView {
    let off = disabled || target.app_id.is_none();
    options
        .into_iter()
        .map(|o| {
            let value = if array {
                serde_json::json!([o.value])
            } else {
                serde_json::json!(o.value)
            };
            action_button(
                "choice",
                o.label,
                off,
                confirm.clone(),
                target.clone(),
                custom_id.clone(),
                value,
            )
        })
        .collect_view()
        .into_any()
}

fn node(c: Component, target: &Target) -> AnyView {
    match c {
        Component::TextDisplay(x) => mfm_view(x.content),
        Component::Container(x) => {
            let color = x.accent_color.map(|c| format!("border-left-color:#{c:06x}")).unwrap_or_default();
            let title = x.title.map(mfm_view);
            let body = if x.spoiler {
                small("ネタバレを含む部品です（Web 版で表示できます）".into())
            } else {
                x.components.into_iter().map(|c| node(c, target)).collect_view().into_any()
            };
            view! { <div class="container" style=color>{title}{body}</div> }.into_any()
        }
        Component::Section(x) => {
            let texts = x.components.into_iter().map(|c| node(c, target)).collect_view();
            let accessory = x.accessory.map(|a| node(*a, target));
            view! { <div class="section"><div class="texts">{texts}</div>{accessory}</div> }.into_any()
        }
        Component::Thumbnail(x) => link(format!("🖼 {}", x.description.unwrap_or_default()), &x.media.url),
        Component::MediaGallery(x) => x
            .items
            .into_iter()
            .enumerate()
            .map(|(i, item)| link(format!("🖼 {}", item.description.unwrap_or_else(|| (i + 1).to_string())), &item.media.url))
            .collect_view()
            .into_any(),
        Component::File(x) => {
            let name = x.name.unwrap_or_else(|| x.file.url.rsplit('/').next().unwrap_or("file").to_string());
            link(format!("📄 {name}"), &x.file.url)
        }
        Component::Separator(x) => {
            if x.divider {
                view! { <hr /> }.into_any()
            } else {
                view! { <div class="gap" /> }.into_any()
            }
        }
        Component::ActionRow(x) => {
            let items = x.components.into_iter().map(|c| node(c, target)).collect_view();
            view! { <div class="row">{items}</div> }.into_any()
        }
        Component::Button(b) => {
            let label = [b.emoji.and_then(|e| e.name), b.label].into_iter().flatten().collect::<Vec<_>>().join(" ");
            if b.style == button_style::LINK {
                // 押せない状態のリンクボタンは、リンクにしない
                if b.disabled {
                    return view! { <button class="button" disabled=true>{label}</button> }.into_any();
                }
                return link(label, b.url.as_deref().unwrap_or_default());
            }
            let class = match b.style {
                button_style::PRIMARY => "button primary",
                button_style::SUCCESS => "button success",
                button_style::DANGER => "button danger",
                _ => "button",
            };
            let off = b.disabled || target.app_id.is_none() || b.custom_id.is_none();
            let value = serde_json::json!(b.value.unwrap_or_default());
            action_button(class, label, off, b.confirm, target.clone(), b.custom_id.unwrap_or_default(), value)
        }
        // 複数を選ぶ select は、選んだ値をまとめて送る必要があるので Web 版で操作する
        Component::StringSelect(s) if s.max_values.unwrap_or(1) > 1 => web_only(),
        Component::StringSelect(s) => choices(target, s.custom_id, s.options, s.disabled, true, s.confirm),
        Component::Overflow(g) | Component::RadioGroup(g) => choices(target, g.custom_id, g.options, g.disabled, false, g.confirm),
        Component::Label(l) => {
            let inner = node(*l.component, target);
            view! { <div class="label"><div class="b">{l.label}</div>{l.description.map(small)}{inner}</div> }.into_any()
        }
        Component::TextInput(_)
        | Component::UserSelect(_)
        | Component::RoleSelect(_)
        | Component::MentionableSelect(_)
        | Component::ChannelSelect(_)
        | Component::ExternalSelect(_)
        | Component::DatePicker(_)
        | Component::TimePicker(_)
        | Component::DateTimePicker(_)
        | Component::CheckboxGroup(_)
        | Component::Checkbox(_)
        | Component::FileUpload(_) => web_only(),
        Component::Video(v) => {
            let url = v.title_url.clone().unwrap_or(v.video_url.clone());
            view! { <div>{link(format!("▶ {}", v.title), &url)}{v.description.map(small)}</div> }.into_any()
        }
        Component::Table(t) => {
            let header = t.header;
            let rows = t
                .rows
                .into_iter()
                .enumerate()
                .map(|(r, row)| {
                    let cells = row
                        .into_iter()
                        .map(|cell| match cell.button {
                            Some(b) => view! { <td>{node(*b, target)}</td> }.into_any(),
                            None if header && r == 0 => view! { <th>{mfm_view(cell.text)}</th> }.into_any(),
                            None => view! { <td>{mfm_view(cell.text)}</td> }.into_any(),
                        })
                        .collect_view();
                    view! { <tr>{cells}</tr> }
                })
                .collect_view();
            view! { <table class="table"><caption>{t.caption}</caption>{rows}</table> }.into_any()
        }
        Component::Alert(a) => view! { <div class=format!("alert {}", a.level.unwrap_or_default())>{mfm_view(a.content)}</div> }.into_any(),
        Component::Card(card) => {
            // 画像は取得せず、リンクで出す（ほかの画像の部品と同じ）
            let images = [card.hero_image, card.icon]
                .into_iter()
                .flatten()
                .map(|m| link("🖼".into(), &m.url))
                .collect_view();
            let texts = [card.title, card.subtitle, card.body, card.subtext]
                .into_iter()
                .flatten()
                .map(mfm_view)
                .collect_view();
            let actions = card.actions.into_iter().map(|a| node(a, target)).collect_view();
            view! { <div class="card">{images}{texts}<div class="row">{actions}</div></div> }.into_any()
        }
        Component::Carousel(c) => {
            let cards = c.cards.into_iter().map(|c| node(c, target)).collect_view();
            view! { <div class="carousel">{cards}</div> }.into_any()
        }
        Component::Chart(c) => {
            let mut lines: Vec<String> = c.segments.iter().map(|s| format!("{}: {}", s.label, s.value)).collect();
            lines.extend(c.series.iter().map(|s| {
                let values = s.data.iter().map(|p| format!("{} {}", p.label, p.value)).collect::<Vec<_>>().join(", ");
                format!("{}: {values}", s.name)
            }));
            let lines = lines.into_iter().map(small).collect_view();
            view! { <figure><figcaption>{c.title}</figcaption>{lines}</figure> }.into_any()
        }
        Component::Grid(g) => {
            let rows = g
                .rows
                .into_iter()
                .map(|row| {
                    let items = row
                        .into_iter()
                        .map(|item| view! { <div style=format!("flex:{}", item.width)>{node(*item.component, target)}</div> })
                        .collect_view();
                    view! { <div class="grid-row">{items}</div> }
                })
                .collect_view();
            view! { <div class="grid">{g.label.map(small)}{rows}</div> }.into_any()
        }
        Component::Plan(p) => {
            let tasks = p.tasks.into_iter().map(task).collect_view();
            view! { <div class="plan"><div class="b">{p.title}</div>{tasks}</div> }.into_any()
        }
        Component::TaskCard(t) => task(t),
        Component::Unsupported(u) => small(u.text),
    }
}

fn task(t: zoo2_client_types::components::TaskCard) -> AnyView {
    let mark = match t.status.as_str() {
        "complete" => "✓",
        "error" => "✕",
        "in_progress" => "…",
        _ => "○",
    };
    view! { <div>{format!("{mark} {}", t.title)}{t.details.map(small)}</div> }.into_any()
}
