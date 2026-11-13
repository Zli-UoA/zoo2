//! クイックスイッチャー。

use gpui::{
    Context, Div, Focusable, FontWeight, MouseButton, MouseUpEvent, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};

use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn open_quick_switcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).create_channel_modal.is_some() {
            return;
        }
        if self.state.read(cx).quick_switcher.is_some() {
            window.focus(&self.quick_switcher_input.focus_handle(cx));
            return;
        }
        self.quick_switcher_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.create_channel_modal.is_some() {
                s.close_create_channel_modal();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_quick_switcher();
            cx.notify();
        });
        window.focus(&self.quick_switcher_input.focus_handle(cx));
    }

    pub(in crate::view) fn render_quick_switcher_modal(&self, cx: &mut Context<Self>) -> Div {
        let should_scroll = self
            .state
            .update(cx, |s, _| s.take_pending_quick_switcher_scroll());
        let state = self.state.read(cx);
        let theme = state.theme();
        let qs = state.quick_switcher.as_ref().unwrap();
        let candidates = state.quick_switcher_candidates();
        let selected_index = qs.selected_index;
        if should_scroll {
            self.quick_switcher_scroll.scroll_to_item(selected_index);
        }

        let items = candidates.iter().enumerate().map(|(i, (ch, label))| {
            let is_selected = i == selected_index;
            let is_dm = ch.is_dm;
            let has_unread = state.unread.contains(&ch.id);
            let icon = if is_dm { "@" } else { "#" };
            let ch_id = ch.id.clone();

            div()
                .w_full()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(if is_selected {
                    theme.bg_selected
                } else {
                    theme.bg_surface
                })
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_muted)
                                .child(icon),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(if is_selected || has_unread {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(theme.text_primary)
                                .child(label.clone()),
                        ),
                )
                .when(has_unread, |d| {
                    d.child(div().w_2().h_2().rounded_full().bg(theme.accent))
                })
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let target = ch_id.clone();
                        this.state.update(cx, |s, cx| {
                            s.close_quick_switcher();
                            s.open_channel(target);
                            cx.notify();
                        });
                    }),
                )
        });

        div()
            .w(px(480.0))
            .max_h(px(400.0))
            .p_4()
            .rounded_lg()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_sm().child("🔍"))
                    .child(div().flex_1().child(self.quick_switcher_input.clone())),
            )
            .child(
                div()
                    .id("quick-switcher-list")
                    .w_full()
                    .max_h(px(280.0))
                    .track_scroll(&self.quick_switcher_scroll)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(items),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_dim)
                    .child("↑↓ で移動、Enter で決定、Esc で閉じる"),
            )
    }
}
