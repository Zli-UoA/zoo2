//! メッセージ部品（Discord Components V2 形式、frontend の `ComponentTree.tsx`）。
//!
//! - 文字は MFM で描画する。container は左に色の帯、section は横に添え物を置く。
//! - 画像・動画は取得せず、説明とリンクで出す（デスクトップには画像のプロキシがなく、置き場所へ直接取りに行かないため）。
//! - ボタンと選択肢（string select・overflow・radio）は押すとアプリへ送る。入力欄・ピッカー・
//!   ユーザーやチャンネルの選択は、デスクトップでは操作できない旨を出す（モーダルとホームは Web で操作する）。

use gpui::{AnyElement, Context, Div, ParentElement, SharedString, Styled, div, prelude::*, px};
use zoo2_client_core::components::{Component, Document, TableCell, button_style};

use crate::fonts;
use crate::i18n::I18n;
use crate::mfm_view::{self, MfmColors};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::{TextSpec, alpha, h_flex, palette, v_flex};
use crate::view::WorkspaceView;

use super::ogp_card::card_color;

/// 操作の送り先。`app_id` がなければ、操作部品は押せない状態で出す（Webhook・Slack の取り込み）。
pub struct Target<'a> {
    pub key: &'a str,
    pub app_id: Option<&'a str>,
    pub message_id: &'a str,
    pub colors: &'a MfmColors,
    pub accounts: &'a zoo2_client_core::store::AccountProfiles,
    pub me: Option<&'a str>,
}

pub fn render_components(
    doc: &Document,
    target: &Target,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let mut n = 0usize;
    v_flex()
        .gap(px(6.0))
        .min_w_0()
        .children(
            doc.components
                .iter()
                .map(|c| node(c, target, &mut n, i18n, cx))
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn text(content: &str, target: &Target, n: &mut usize) -> AnyElement {
    *n += 1;
    mfm_view::render_mfm_accounts_with_colors(
        &format!("{}-c{n}", target.key),
        content,
        target.me,
        target.colors,
        target.accounts,
    )
    .into_any_element()
}

fn small(s: impl Into<SharedString>, cx: &mut Context<WorkspaceView>) -> Div {
    let t = palette(cx).t;
    let xs = TextSpec::XS;
    div()
        .text_size(xs.size())
        .line_height(xs.line())
        .text_color(t.muted_foreground)
        .child(s.into())
}

/// https の URL だけを開くリンク。
fn link(id: String, label: String, url: &str, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let t = palette(cx).t;
    if !zoo2_client_core::components::validate::is_https_url(url) {
        return div().child(label).into_any_element();
    }
    let url = url.to_string();
    div()
        .id(SharedString::from(id))
        .cursor_pointer()
        .text_color(t.link)
        .hover(|s| s.underline())
        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
        .child(label)
        .into_any_element()
}

fn node(
    c: &Component,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    *n += 1;
    let id = format!("{}-n{}", target.key, *n);
    match c {
        Component::TextDisplay(x) => text(&x.content, target, n),
        Component::Container(x) => {
            let stripe = x
                .accent_color
                .and_then(|v| card_color(&format!("#{v:06x}")))
                .unwrap_or(t.brand);
            let mut body = v_flex().gap(px(6.0)).min_w_0().flex_1();
            if let Some(title) = &x.title {
                body = body.child(div().font(fonts::sans(600.0)).child(text(title, target, n)));
            }
            if x.spoiler {
                body = body.child(small(i18n.t("apps.components.spoiler"), cx));
            } else {
                for child in &x.components {
                    body = body.child(node(child, target, n, i18n, cx));
                }
            }
            div()
                .max_w(px(560.0))
                .rounded_md()
                .border_l(px(4.0))
                .border_color(stripe)
                .bg(t.card)
                .child(div().p_3().min_w_0().child(body))
                .into_any_element()
        }
        Component::Section(x) => {
            let texts = v_flex().flex_1().min_w_0().gap(px(4.0)).children(
                x.components
                    .iter()
                    .map(|c| node(c, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            );
            let accessory = x.accessory.as_deref().map(|a| node(a, target, n, i18n, cx));
            h_flex()
                .items_start()
                .gap_3()
                .child(texts)
                .children(accessory)
                .into_any_element()
        }
        Component::Thumbnail(x) => link(
            id,
            format!("🖼 {}", x.description.clone().unwrap_or_default()),
            &x.media.url,
            cx,
        ),
        Component::MediaGallery(x) => v_flex()
            .children(
                x.items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let label = format!(
                            "🖼 {}",
                            item.description
                                .clone()
                                .unwrap_or_else(|| format!("{}", i + 1))
                        );
                        link(format!("{id}-{i}"), label, &item.media.url, cx)
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::File(x) => link(
            id,
            format!(
                "📄 {}",
                x.name.clone().unwrap_or_else(|| x
                    .file
                    .url
                    .rsplit('/')
                    .next()
                    .unwrap_or("file")
                    .to_string())
            ),
            &x.file.url,
            cx,
        ),
        Component::Separator(x) => {
            if x.divider {
                div()
                    .my(px(if x.spacing == 2 { 8.0 } else { 2.0 }))
                    .h(px(1.0))
                    .bg(t.border)
                    .into_any_element()
            } else {
                div()
                    .h(px(if x.spacing == 2 { 12.0 } else { 4.0 }))
                    .into_any_element()
            }
        }
        Component::ActionRow(x) => h_flex()
            .flex_wrap()
            .gap_2()
            .children(
                x.components
                    .iter()
                    .map(|c| node(c, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Button(b) => {
            let label = [
                b.emoji.as_ref().and_then(|e| e.name.clone()),
                b.label.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            if b.style == button_style::LINK {
                let url = b.url.clone().unwrap_or_default();
                return Button::new(SharedString::from(id))
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .label(label)
                    .disabled(!zoo2_client_core::components::validate::is_https_url(&url))
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                    .into_any_element();
            }
            let variant = match b.style {
                button_style::PRIMARY => ButtonVariant::Default,
                button_style::SUCCESS => ButtonVariant::Secondary,
                button_style::DANGER => ButtonVariant::Destructive,
                _ => ButtonVariant::Outline,
            };
            let action = action(
                target,
                b.custom_id.clone(),
                serde_json::json!(b.value.clone().unwrap_or_default()),
            );
            Button::new(SharedString::from(id))
                .variant(variant)
                .size(ButtonSize::Sm)
                .label(label)
                .disabled(b.disabled || action.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some((app, message, custom, value)) = action.clone() {
                        this.state.update(cx, |s, cx| {
                            s.invoke_component(&app, &message, &custom, value);
                            cx.notify();
                        });
                    }
                }))
                .into_any_element()
        }
        Component::StringSelect(s) => choices(
            &id,
            target,
            &s.custom_id,
            &s.options,
            s.disabled,
            s.placeholder.as_deref(),
            true,
            cx,
        ),
        Component::Overflow(g) | Component::RadioGroup(g) => choices(
            &id,
            target,
            &g.custom_id,
            &g.options,
            g.disabled,
            None,
            false,
            cx,
        ),
        Component::Label(l) => v_flex()
            .gap_1()
            .child(div().font(fonts::sans(500.0)).child(l.label.clone()))
            .children(l.description.clone().map(|d| small(d, cx)))
            .child(node(&l.component, target, n, i18n, cx))
            .into_any_element(),
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
        | Component::FileUpload(_) => {
            small(i18n.t("apps.components.webOnly"), cx).into_any_element()
        }
        Component::Video(v) => v_flex()
            .child(link(
                id,
                format!("▶ {}", v.title),
                v.title_url.as_deref().unwrap_or(&v.video_url),
                cx,
            ))
            .children(v.description.clone().map(|d| small(d, cx)))
            .into_any_element(),
        Component::Table(x) => table(x, target, n, i18n, cx),
        Component::Alert(a) => div()
            .rounded_md()
            .border_1()
            .border_color(match a.level.as_deref() {
                Some("error") => t.destructive,
                _ => t.border,
            })
            .bg(alpha(t.muted, 0.5))
            .px_3()
            .py_2()
            .child(text(&a.content, target, n))
            .into_any_element(),
        Component::Card(card) => card_view(card, target, n, i18n, cx),
        Component::Carousel(c) => h_flex()
            .gap_2()
            .flex_wrap()
            .children(
                c.cards
                    .iter()
                    .map(|x| node(x, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Chart(c) => {
            let mut lines = vec![div().font(fonts::sans(600.0)).child(c.title.clone())];
            for s in &c.segments {
                lines.push(small(format!("{}: {}", s.label, s.value), cx));
            }
            for s in &c.series {
                let values = s
                    .data
                    .iter()
                    .map(|p| format!("{} {}", p.label, p.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(small(format!("{}: {values}", s.name), cx));
            }
            v_flex().children(lines).into_any_element()
        }
        Component::Grid(g) => v_flex()
            .gap_2()
            .children(
                g.label
                    .clone()
                    .map(|l| div().font(fonts::sans(600.0)).child(l)),
            )
            .children(
                g.rows
                    .iter()
                    .map(|row| {
                        h_flex().items_start().gap_3().children(
                            row.iter()
                                .map(|item| {
                                    div().flex_1().min_w_0().child(node(
                                        &item.component,
                                        target,
                                        n,
                                        i18n,
                                        cx,
                                    ))
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Plan(p) => v_flex()
            .child(div().font(fonts::sans(600.0)).child(p.title.clone()))
            .children(
                p.tasks
                    .iter()
                    .map(|task| task_line(task, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::TaskCard(task) => task_line(task, cx).into_any_element(),
        Component::Unsupported(u) => {
            if u.text.is_empty() {
                div().into_any_element()
            } else {
                small(u.text.clone(), cx).into_any_element()
            }
        }
    }
}

type Action = Option<(String, String, String, serde_json::Value)>;

fn action(target: &Target, custom_id: Option<String>, value: serde_json::Value) -> Action {
    Some((
        target.app_id?.to_string(),
        target.message_id.to_string(),
        custom_id?,
        value,
    ))
}

/// 選択肢を小さなボタンで並べる。押すとその値をアプリへ送る（select は配列、overflow と radio は文字列）。
#[allow(clippy::too_many_arguments)]
fn choices(
    id: &str,
    target: &Target,
    custom_id: &str,
    options: &[zoo2_client_core::components::SelectOption],
    disabled: bool,
    placeholder: Option<&str>,
    array: bool,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let buttons = options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let value = if array {
                serde_json::json!([o.value])
            } else {
                serde_json::json!(o.value)
            };
            let act = action(target, Some(custom_id.to_string()), value);
            Button::new(SharedString::from(format!("{id}-o{i}")))
                .variant(if o.default {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Xs)
                .label(o.label.clone())
                .disabled(disabled || act.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some((app, message, custom, value)) = act.clone() {
                        this.state.update(cx, |s, cx| {
                            s.invoke_component(&app, &message, &custom, value);
                            cx.notify();
                        });
                    }
                }))
        })
        .collect::<Vec<_>>();
    v_flex()
        .gap_1()
        .children(placeholder.map(|p| small(p.to_string(), cx)))
        .child(h_flex().flex_wrap().gap_1().children(buttons))
        .into_any_element()
}

fn table(
    x: &zoo2_client_core::components::Table,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    let cell = |c: &TableCell, n: &mut usize, cx: &mut Context<WorkspaceView>| -> AnyElement {
        match &c.button {
            Some(b) => node(b, target, n, i18n, cx),
            None => text(&c.text, target, n),
        }
    };
    let mut rows = Vec::new();
    for (r, row) in x.rows.iter().take(50).enumerate() {
        let cells = row
            .iter()
            .map(|c| div().w(px(120.0)).px_2().child(cell(c, n, cx)))
            .collect::<Vec<_>>();
        let mut line = h_flex()
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .children(cells);
        if x.header && r == 0 {
            line = line.font(fonts::sans(600.0));
        }
        rows.push(line);
    }
    v_flex()
        .children(x.caption.clone().map(|c| small(c, cx)))
        .children(rows)
        .into_any_element()
}

fn card_view(
    card: &zoo2_client_core::components::Card,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    let mut body = v_flex()
        .gap_1()
        .p_2()
        .w(px(240.0))
        .rounded_md()
        .border_1()
        .border_color(t.border)
        .bg(t.card);
    if let Some(title) = &card.title {
        body = body.child(div().font(fonts::sans(600.0)).child(text(title, target, n)));
    }
    for s in [&card.subtitle, &card.body, &card.subtext]
        .into_iter()
        .flatten()
    {
        body = body.child(text(s, target, n));
    }
    let actions = card
        .actions
        .iter()
        .map(|a| node(a, target, n, i18n, cx))
        .collect::<Vec<_>>();
    body.child(h_flex().gap_1().flex_wrap().children(actions))
        .into_any_element()
}

fn task_line(
    task: &zoo2_client_core::components::TaskCard,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let mark = match task.status.as_str() {
        "complete" => "✓",
        "error" => "✕",
        "in_progress" => "…",
        _ => "○",
    };
    v_flex()
        .child(format!("{mark} {}", task.title))
        .children(task.details.clone().map(|d| small(d, cx)))
}
