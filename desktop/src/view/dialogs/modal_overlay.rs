//! モーダルの背景 (frontend の `ui/dialog.tsx` のオーバーレイに対応)。

use gpui::{Context, Div, MouseButton, ParentElement, Styled, Window, div, prelude::*, rgba};

use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn render_modal_overlay(
        &self,
        has_create_modal: bool,
        has_quick_switcher: bool,
        has_emoji_picker: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x00000077))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .justify_center()
            .when(has_create_modal, |d| {
                d.child(self.render_create_channel_modal(window, cx))
            })
            .when(has_quick_switcher, |d| {
                d.child(self.render_quick_switcher_modal(cx))
            })
            .when(has_emoji_picker, |d| d.child(self.render_emoji_picker(cx)))
    }
}
