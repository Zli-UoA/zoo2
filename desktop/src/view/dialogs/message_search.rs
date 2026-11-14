//! メッセージ検索バー。

use gpui::{Context, Div, Focusable, ParentElement, Styled, Window, div, prelude::*, px};

use crate::icons::Icon;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::input::Input;
use crate::ui::{TextSpec, h_flex, icon_colored, palette};
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

    /// チャンネル内のメッセージ検索。Web 版には対応する画面がない。
    /// 一致したメッセージをタイムラインで見ながら操作するので、ダイアログにはせず、
    /// タイムラインの上の帯に出す。部品と寸法は `ChannelBrowserDialog.tsx` の検索欄に合わせる。
    pub(in crate::view) fn render_message_search_bar(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let xs = TextSpec::XS;
        let state = self.state.read(cx);
        let ms = state.message_search.as_ref().unwrap();
        let count = ms.matches.len();
        let count_text = if ms.query.trim().is_empty() {
            String::new()
        } else if count == 0 {
            i18n.t_count("messages.count", 0, &[])
        } else {
            format!("{}/{}", ms.current_match_index + 1, count)
        };

        // `shrink-0 border-b bg-background px-4 py-2`
        h_flex()
            .w_full()
            .flex_none()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(8.0))
            .bg(t.background)
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Input::new()
                            .focus_handle(&self.message_search_input.focus_handle(cx))
                            .pl(px(32.0))
                            .child(self.message_search_input.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(10.0))
                            .flex()
                            .items_center()
                            .child(icon_colored(Icon::Search, 16.0, t.muted_foreground)),
                    ),
            )
            .when(!count_text.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.muted_foreground)
                        .child(count_text),
                )
            })
            .when(count > 0, |d| {
                d.child(
                    Button::new("btn-search-prev")
                        .focus_handle(&self.search_prev_focus)
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::IconSm)
                        .icon_start(Icon::ChevronUp)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |s, cx| {
                                s.search_select_prev();
                                cx.notify();
                            });
                        })),
                )
                .child(
                    Button::new("btn-search-next")
                        .focus_handle(&self.search_next_focus)
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::IconSm)
                        .icon_start(Icon::ChevronDown)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |s, cx| {
                                s.search_select_next();
                                cx.notify();
                            });
                        })),
                )
            })
            .child(
                Button::new("btn-search-close")
                    .focus_handle(&self.search_close_focus)
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon_start(Icon::X)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.message_search_input
                            .update(cx, |input, cx| input.clear(cx));
                        this.state.update(cx, |s, cx| {
                            s.close_message_search();
                            cx.notify();
                        });
                    })),
            )
    }
}
