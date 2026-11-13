//! 削除確認バー (frontend の `ui/confirm-action-dialog.tsx` に対応)。

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseUpEvent, ParentElement, Styled, div, prelude::*,
};

use crate::theme::Theme;
use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn render_delete_confirm_bar(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .px_4()
            .py_2()
            .bg(theme.bg_secondary)
            .border_b_1()
            .border_color(theme.danger)
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.warning)
                    .child("選択したメッセージを削除しますか？"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .id("btn-confirm-delete-yes")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme.danger)
                            .hover(|s| s.cursor_pointer())
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("削除する")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.confirm_delete();
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("btn-confirm-delete-no")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.border)
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("キャンセル")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.cancel_delete();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
    }
}
