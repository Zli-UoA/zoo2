//! リンクプレビュー (OGP カード)。

use gpui::{
    Context, Div, FocusHandle, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    Styled, div, prelude::*, px,
};
use zoo2_client_core::electric::rows::CardData;

use crate::theme;
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

pub fn render_ogp_card(
    card: &CardData,
    card_key: &str,
    is_expanded: bool,
    card_focus_handles: &std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
    theme: &theme::Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let border_color = if let Some(color_hex) = &card.color {
        let clean = color_hex.trim_start_matches('#');
        if let Ok(c) = u32::from_str_radix(clean, 16) {
            theme::hex_rgb(c)
        } else {
            theme.border_accent
        }
    } else {
        theme.border_accent
    };

    let title = card.title.clone();
    let url = card.url.clone();
    let desc = card.description.clone();
    let meta_parts: Vec<&str> = [card.source.as_str(), card.context.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    let meta_text = meta_parts.join(" · ");
    let badge_text = card.badge.clone();
    let author = card.author.clone();

    let open_target = crate::login::validate_http_url(&card.url)
        .map(|u| u.to_string())
        .unwrap_or_default();

    let fields_div = if let Some(fields) = &card.fields {
        if !fields.is_empty() {
            let field_items = fields.iter().enumerate().map(|(idx, f)| {
                let f_title = f.title.clone();
                let f_val = f.value.clone();
                let is_code = f.code.unwrap_or(false);
                let f_url = f
                    .url
                    .as_deref()
                    .and_then(crate::login::validate_http_url)
                    .map(|u| u.to_string());

                let mut val_view = div()
                    .text_xs()
                    .text_color(if f_url.is_some() {
                        theme.text_link
                    } else {
                        theme.text_primary
                    })
                    .child(f_val);

                if is_code {
                    val_view = val_view
                        .px_1()
                        .py_0p5()
                        .rounded_sm()
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.border);
                }

                if let Some(target) = f_url {
                    let field_focus = card_focus_handles
                        .borrow_mut()
                        .entry(format!("{card_key}:field:{idx}"))
                        .or_insert_with(|| cx.focus_handle().tab_stop(true))
                        .clone();
                    let target_for_key = target.clone();
                    val_view = val_view
                        .track_focus(&field_focus)
                        .focus(|s| s.bg(theme.bg_hover))
                        .hover(|s| s.cursor_pointer())
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
                                cx.open_url(&target_for_key);
                            }
                        }));
                }

                let is_short = f.short != Some(false);
                let mut item_container = div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_muted)
                            .child(f_title),
                    )
                    .child(val_view);
                if is_short {
                    item_container = item_container.min_w(px(120.0)).flex_1();
                } else {
                    item_container = item_container.w_full();
                }
                item_container
            });

            Some(
                div()
                    .mt_1p5()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(field_items),
            )
        } else {
            None
        }
    } else {
        None
    };

    let desc_element = if !desc.is_empty() {
        if is_ogp_description_long(&desc) {
            let key = card_key.to_string();
            let key_kd = key.clone();
            let toggle_focus = card_focus_handles
                .borrow_mut()
                .entry(format!("{card_key}:toggle"))
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            let toggle_btn = if is_expanded {
                div()
                    .track_focus(&toggle_focus)
                    .focus(|s| s.bg(theme.bg_hover))
                    .text_xs()
                    .text_color(theme.text_link)
                    .hover(|s| s.cursor_pointer())
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                            cx.stop_propagation();
                            this.expanded_cards.remove(&key);
                            cx.notify();
                        }),
                    )
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                        if is_activate_key(&ev.keystroke.key) {
                            cx.stop_propagation();
                            this.expanded_cards.remove(&key_kd);
                            cx.notify();
                        }
                    }))
                    .child("折りたたむ")
            } else {
                div()
                    .track_focus(&toggle_focus)
                    .focus(|s| s.bg(theme.bg_hover))
                    .text_xs()
                    .text_color(theme.text_link)
                    .hover(|s| s.cursor_pointer())
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                            cx.stop_propagation();
                            this.expanded_cards.insert(key.clone());
                            cx.notify();
                        }),
                    )
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                        if is_activate_key(&ev.keystroke.key) {
                            cx.stop_propagation();
                            this.expanded_cards.insert(key_kd.clone());
                            cx.notify();
                        }
                    }))
                    .child("さらに表示")
            };

            let display_text = if is_expanded {
                desc.clone()
            } else {
                truncate_ogp_description(&desc)
            };

            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(display_text),
                    )
                    .child(toggle_btn),
            )
        } else {
            Some(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(desc.clone()),
            )
        }
    } else {
        None
    };

    let is_clickable = !open_target.is_empty();
    let card_link_focus = card_focus_handles
        .borrow_mut()
        .entry(format!("{card_key}:card"))
        .or_insert_with(|| cx.focus_handle().tab_stop(true))
        .clone();
    let target_for_card_key = open_target.clone();

    div()
        .mt_1p5()
        .rounded_md()
        .bg(theme.bg_surface)
        .border_1()
        .border_color(theme.border)
        .flex()
        .flex_row()
        .overflow_hidden()
        .when(is_clickable, |d| {
            d.track_focus(&card_link_focus)
                .focus(|s| s.border_color(theme.border_accent))
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                        cx.open_url(&open_target);
                    }),
                )
                .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                    if is_activate_key(&ev.keystroke.key) {
                        cx.stop_propagation();
                        cx.open_url(&target_for_card_key);
                    }
                }))
        })
        .child(div().w(px(4.0)).bg(border_color))
        .child(
            div()
                .flex_1()
                .p_2p5()
                .flex()
                .flex_col()
                .gap_1()
                .when(!meta_text.is_empty() || !badge_text.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .when(!meta_text.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(meta_text),
                                )
                            })
                            .when(!badge_text.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(theme.border)
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(badge_text),
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text_link)
                        .child(title),
                )
                .when_some(desc_element, |d, el| d.child(el))
                .when_some(fields_div, |d, f| d.child(f))
                .when(!author.is_empty(), |d| {
                    d.child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(author),
                    )
                })
                .when(!url.is_empty(), |d| {
                    d.child(div().text_xs().text_color(theme.text_dim).child(url))
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ogp_hex_color() {
        let c = theme::hex_rgb(0xff0000);
        assert!((c.r - 1.0).abs() < 0.01);
        assert!(c.g.abs() < 0.01);
        assert!(c.b.abs() < 0.01);
        assert!((c.a - 1.0).abs() < 0.01);
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
