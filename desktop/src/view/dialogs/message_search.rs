//! メッセージ検索バー。

use gpui::{
    Context, Div, Focusable, MouseButton, MouseUpEvent, ParentElement, Styled, Window, div,
    prelude::*,
};

use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn toggle_message_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).create_channel_modal.is_some() {
            return;
        }
        let is_open = self.state.read(cx).message_search.is_some();
        if is_open {
            self.message_search_input
                .update(cx, |input, cx| input.clear(cx));
            self.state.update(cx, |s, cx| {
                s.close_message_search();
                cx.notify();
            });
        } else {
            self.open_message_search(window, cx);
        }
    }

    pub(in crate::view) fn open_message_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).create_channel_modal.is_some() {
            return;
        }
        self.message_search_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.quick_switcher.is_some() {
                s.close_quick_switcher();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_message_search();
            s.pending_focus_composer = false;
            cx.notify();
        });
        window.focus(&self.message_search_input.focus_handle(cx));
    }

    pub(in crate::view) fn render_message_search_bar(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let ms = state.message_search.as_ref().unwrap();
        let count = ms.matches.len();
        let count_text = if ms.query.trim().is_empty() {
            String::new()
        } else if count == 0 {
            "0 件".to_string()
        } else {
            format!("{}/{} 件", ms.current_match_index + 1, count)
        };

        div()
            .w_full()
            .px_4()
            .py_2()
            .bg(theme.bg_surface)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(div().text_sm().child("🔍"))
            .child(div().flex_1().child(self.message_search_input.clone()))
            .when(!count_text.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(if count > 0 {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                })
                                .child(count_text),
                        )
                        .when(count > 0, |d| {
                            d.child(
                                div()
                                    .id("btn-search-prev")
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_primary)
                                    .child("↑")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.search_select_prev();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("btn-search-next")
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_primary)
                                    .child("↓")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.search_select_next();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                        }),
                )
            })
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child("閉じる (Esc)")
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.message_search_input
                                .update(cx, |input, cx| input.clear(cx));
                            this.state.update(cx, |s, cx| {
                                s.close_message_search();
                                cx.notify();
                            });
                        }),
                    ),
            )
    }
}
