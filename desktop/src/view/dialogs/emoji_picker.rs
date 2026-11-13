//! 絵文字ピッカー。

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseUpEvent, ParentElement, StatefulInteractiveElement,
    Styled, div, prelude::*, px,
};

use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(crate) fn render_emoji_picker(&self, cx: &mut Context<Self>) -> Div {
        let (selected_idx, should_scroll, theme) = self.state.update(cx, |s, _| {
            let scroll = s.take_pending_emoji_picker_scroll();
            (s.emoji_picker_selected, scroll, s.theme())
        });
        if should_scroll {
            self.emoji_picker_scroll.scroll_to_item(selected_idx);
        }

        let filtered = self.state.read(cx).filtered_emojis();

        let buttons = filtered.into_iter().enumerate().map(|(idx, e)| {
            let emoji_str = e.to_string();
            let is_selected = idx == selected_idx;
            div()
                .w_7()
                .h_7()
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .when(is_selected, |d| {
                    d.bg(theme.bg_selected)
                        .border_1()
                        .border_color(theme.accent)
                })
                .when(!is_selected, |d| {
                    d.hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                })
                .text_base()
                .child(e)
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let em = emoji_str.clone();
                        let range = this.composer_input.read(cx).selected_range();
                        this.state.update(cx, |s, cx| {
                            s.insert_emoji(&em, range);
                            cx.notify();
                        });
                    }),
                )
        });

        div()
            .w(px(320.0))
            .max_h(px(280.0))
            .p_3()
            .rounded_md()
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("絵文字を選択"),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("✕")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.close_emoji_picker();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .id("emoji-picker-list")
                    .track_scroll(&self.emoji_picker_scroll)
                    .w_full()
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_1()
                    .children(buttons),
            )
    }
}
