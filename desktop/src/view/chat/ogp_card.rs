//! 連携カード (frontend の `MessageCard.tsx`)。サーバーが `card` 列に入れる、連携ごとの JSON。
//!
//! `LinkPreviews.tsx` (本文の URL から summaly で作るプレビュー) はまだ写していない
//! (デスクトップに summaly のクライアントと、そのエンドポイントを引く処理がない)。
//! `attachment` 表示 (`app_attachments`) と `app_blocks` も、デスクトップのデータに無いので写していない。

use gpui::{
    Context, Div, ElementId, FocusHandle, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    Rgba, SharedString, Styled, div, prelude::*, px, relative,
};
use zoo2_client_core::electric::rows::CardData;

use crate::fonts;
use crate::i18n::I18n;
use crate::icons::Icon;
use crate::tokens::{hex, radius};
use crate::ui::{TextSpec, alpha, h_flex, icon_colored, palette, shadow_sm, v_flex};
use crate::view::{WorkspaceView, is_activate_key};

pub const CARD_DESC_MAX_CHARS: usize = 300;
pub const CARD_DESC_MAX_LINES: usize = 8;

pub fn is_ogp_description_long(desc: &str) -> bool {
    desc.lines().count() > CARD_DESC_MAX_LINES || desc.chars().count() > CARD_DESC_MAX_CHARS
}

pub fn truncate_ogp_description(desc: &str) -> String {
    let mut truncated = String::new();
    for (i, line) in desc.lines().enumerate() {
        if i >= CARD_DESC_MAX_LINES {
            break;
        }
        if !truncated.is_empty() {
            truncated.push('\n');
        }
        truncated.push_str(line);
    }
    if truncated.chars().count() > CARD_DESC_MAX_CHARS {
        truncated = truncated.chars().take(CARD_DESC_MAX_CHARS).collect();
    }
    format!("{}...", truncated.trim_end())
}

/// `cardColor`。`good` `warning` `danger` の名前か、`#rrggbb`。
pub fn card_color(value: &str) -> Option<Rgba> {
    match value {
        "good" => return Some(hex(0x2eb886)),
        "warning" => return Some(hex(0xdaa038)),
        "danger" => return Some(hex(0xa30200)),
        _ => {}
    }
    let digits = value.strip_prefix('#').unwrap_or(value);
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        u32::from_str_radix(digits, 16).ok().map(hex)
    } else {
        None
    }
}

/// `ExternalLink` の見た目で、押すとブラウザで開く文字。
#[allow(clippy::too_many_arguments)]
fn link_text(
    id: String,
    focus: FocusHandle,
    text: String,
    target: String,
    t: &crate::tokens::Tokens,
    cx: &mut Context<WorkspaceView>,
) -> gpui::Stateful<Div> {
    let target_key = target.clone();
    let ring = alpha(t.ring, 0.5);
    div()
        .id(ElementId::Name(SharedString::from(id)))
        .track_focus(&focus)
        .focus(move |s| s.bg(alpha(ring, 0.3)))
        .cursor_pointer()
        .hover(|s| s.underline())
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                cx.stop_propagation();
                cx.open_url(&target);
            }),
        )
        .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
            if is_activate_key(&ev.keystroke.key) {
                cx.stop_propagation();
                cx.open_url(&target_key);
            }
        }))
        .child(text)
}

/// `https` の URL だけを開く (`safeHref`)。
fn safe_href(url: &str) -> Option<String> {
    crate::login::validate_http_url(url)
        .filter(|u| u.scheme() == "https")
        .map(|u| u.to_string())
}

pub fn render_ogp_card(
    card: &CardData,
    card_key: &str,
    is_expanded: bool,
    card_focus_handles: &std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let t = palette(cx).t;
    let sm = TextSpec::SM;
    let xs = TextSpec::XS;
    let stripe = card
        .color
        .as_deref()
        .and_then(card_color)
        .unwrap_or(t.brand);
    let focus_for = |key: String, cx: &mut Context<WorkspaceView>| {
        card_focus_handles
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    };

    let meta_text = [card.source.as_str(), card.context.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let has_meta = !meta_text.is_empty() || !card.badge.is_empty();

    // `break-words font-semibold`。URL があればリンク。
    let title = match safe_href(&card.url) {
        Some(target) => {
            let focus = focus_for(format!("{card_key}:card"), cx);
            link_text(
                format!("card-title-{card_key}"),
                focus,
                card.title.clone(),
                target,
                &t,
                cx,
            )
            .into_any_element()
        }
        None => div().child(card.title.clone()).into_any_element(),
    };

    let desc = (!card.description.is_empty()).then(|| {
        let long = is_ogp_description_long(&card.description);
        let text = if long && !is_expanded {
            truncate_ogp_description(&card.description)
        } else {
            card.description.clone()
        };
        let toggle = long.then(|| {
            let focus = focus_for(format!("{card_key}:toggle"), cx);
            let key = card_key.to_string();
            let key_kd = key.clone();
            let label = i18n.t(if is_expanded {
                "cards.showLess"
            } else {
                "cards.showMore"
            });
            let toggle_card = move |this: &mut WorkspaceView, key: &str| {
                if !this.expanded_cards.remove(key) {
                    this.expanded_cards.insert(key.to_string());
                }
            };
            // `Button variant="link" size="sm" className="mt-1 px-0"`
            h_flex()
                .id(ElementId::Name(SharedString::from(format!(
                    "card-toggle-{card_key}"
                ))))
                .track_focus(&focus)
                .focus(|s| s.underline())
                .mt(px(4.0))
                .h(px(28.0))
                .gap(px(4.0))
                .text_size(xs.size())
                .line_height(xs.line())
                .font(fonts::sans(500.0))
                .text_color(t.primary)
                .cursor_pointer()
                .hover(|s| s.underline())
                .child(icon_colored(
                    if is_expanded {
                        Icon::ChevronUp
                    } else {
                        Icon::ChevronDown
                    },
                    12.0,
                    t.primary,
                ))
                .child(label)
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        cx.stop_propagation();
                        toggle_card(this, &key);
                        cx.notify();
                    }),
                )
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                    if is_activate_key(&ev.keystroke.key) {
                        cx.stop_propagation();
                        if !this.expanded_cards.remove(&key_kd) {
                            this.expanded_cards.insert(key_kd.clone());
                        }
                        cx.notify();
                    }
                }))
        });
        // `mt-2`、本文は `whitespace-pre-wrap break-words text-muted-foreground`
        v_flex()
            .items_start()
            .mt(px(8.0))
            .child(div().text_color(t.muted_foreground).child(text))
            .children(toggle)
    });

    let fields = card
        .fields
        .as_ref()
        .filter(|f| !f.is_empty())
        .map(|fields| {
            let items: Vec<Div> = fields
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let href = f.url.as_deref().and_then(safe_href);
                    let value_text = f.value.clone();
                    let value: gpui::AnyElement = if f.code.unwrap_or(false) {
                        // `rounded border bg-muted px-1 py-0.5 font-mono text-xs break-all`
                        div()
                            .px(px(4.0))
                            .py(px(2.0))
                            .rounded(radius::SM)
                            .border_1()
                            .border_color(t.border)
                            .bg(t.muted)
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .child(value_text)
                            .into_any_element()
                    } else {
                        div().child(value_text).into_any_element()
                    };
                    let value = match href {
                        Some(target) => {
                            let focus = focus_for(format!("{card_key}:field:{idx}"), cx);
                            let shown = div().child(value);
                            let target_key = target.clone();
                            shown
                                .id(ElementId::Name(SharedString::from(format!(
                                    "card-field-{card_key}-{idx}"
                                ))))
                                .track_focus(&focus)
                                .cursor_pointer()
                                .hover(|s| s.underline())
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                                        cx.stop_propagation();
                                        cx.open_url(&target);
                                    }),
                                )
                                .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                                    if is_activate_key(&ev.keystroke.key) {
                                        cx.stop_propagation();
                                        cx.open_url(&target_key);
                                    }
                                }))
                                .into_any_element()
                        }
                        None => value,
                    };
                    let item = v_flex()
                        .items_start()
                        .min_w_0()
                        .child(
                            // `dt mb-1 font-semibold`
                            div()
                                .mb(px(4.0))
                                .font(fonts::sans(600.0))
                                .child(f.title.clone()),
                        )
                        .child(value);
                    // `grid-cols-2 gap-x-6`。`short` でなければ 2 列ぶん。
                    if f.short == Some(true) {
                        item.w(relative(0.5)).pr(px(12.0))
                    } else {
                        item.w_full()
                    }
                })
                .collect();
            // `mt-3 grid gap-y-3`
            div()
                .mt(px(12.0))
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_y(px(12.0))
                .children(items)
        });

    // `@container my-1 max-w-xl overflow-hidden rounded-lg border border-l-4 bg-card p-3 text-sm shadow-sm`
    div()
        .relative()
        .my(px(4.0))
        .max_w(px(576.0))
        .overflow_hidden()
        .rounded(radius::LG)
        .border_1()
        .border_color(t.border)
        .bg(t.card)
        .py(px(12.0))
        .pr(px(12.0))
        .pl(px(15.0))
        .text_size(sm.size())
        .line_height(sm.line())
        .shadow(shadow_sm())
        .child(
            div()
                .absolute()
                .top(px(-1.0))
                .bottom(px(-1.0))
                .left(px(-1.0))
                .w(px(4.0))
                .bg(stripe),
        )
        .when(has_meta, |d| {
            d.child(
                // `mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground`
                h_flex()
                    .mb(px(8.0))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(t.muted_foreground)
                    // `gap-2` は `ml-auto` のバッジがあると効かないので、アイコンの右に余白を置く。
                    .child(div().flex_none().mr(px(8.0)).child(icon_colored(
                        Icon::SquareArrowOutUpRight,
                        16.0,
                        t.muted_foreground,
                    )))
                    .child(div().min_w_0().child(meta_text.clone()))
                    .when(!card.badge.is_empty(), |d| {
                        // `ml-auto rounded border px-1.5 py-0.5`
                        d.child(
                            div()
                                .ml_auto()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(radius::SM)
                                .border_1()
                                .border_color(t.border)
                                .child(card.badge.clone()),
                        )
                    }),
            )
        })
        .child(
            div()
                .font(fonts::sans(600.0))
                .text_color(t.card_foreground)
                .child(title),
        )
        .children(desc)
        .children(fields)
        .when(!card.author.is_empty(), |d| {
            // `mt-2 text-xs text-muted-foreground`
            d.child(
                div()
                    .mt(px(8.0))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(t.muted_foreground)
                    .child(card.author.clone()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_color_accepts_names_and_hex() {
        assert_eq!(card_color("good"), Some(crate::tokens::hex(0x2eb886)));
        assert_eq!(card_color("warning"), Some(crate::tokens::hex(0xdaa038)));
        assert_eq!(card_color("danger"), Some(crate::tokens::hex(0xa30200)));
        assert_eq!(card_color("#ff0000"), Some(crate::tokens::hex(0xff0000)));
        assert_eq!(card_color("ff0000"), Some(crate::tokens::hex(0xff0000)));
        assert_eq!(card_color("red"), None);
        assert_eq!(card_color("#fff"), None);
        // `+` 付きなどの `from_str_radix` が通してしまう形は弾く。
        assert_eq!(card_color("+ff000"), None);
    }

    #[test]
    fn ogp_description_length_and_truncation() {
        let short = "短い説明文です。";
        assert!(!is_ogp_description_long(short));

        // 301文字
        let long_chars = "あ".repeat(301);
        assert!(is_ogp_description_long(&long_chars));
        let truncated_chars = truncate_ogp_description(&long_chars);
        assert_eq!(truncated_chars.chars().count(), 303); // 300文字 + "..."
        assert!(truncated_chars.ends_with("..."));

        // 9行
        let long_lines = (1..=9)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(is_ogp_description_long(&long_lines));
        let truncated_lines = truncate_ogp_description(&long_lines);
        assert_eq!(truncated_lines.lines().count(), 8);
        assert!(truncated_lines.ends_with("..."));
    }
}
