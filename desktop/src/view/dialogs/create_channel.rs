//! チャンネル作成ダイアログ (`CreateChannelDialog`)。

use gpui::{
    Context, Div, Focusable, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    Styled, Window, div, prelude::*, px,
};

use crate::view::{WorkspaceView, is_activate_key};

impl WorkspaceView {
    pub(in crate::view) fn open_create_channel_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).is_current_user_guest() {
            return;
        }
        if self.state.read(cx).create_channel_modal.is_some() {
            window.focus(&self.create_channel_name_input.focus_handle(cx));
            return;
        }
        self.create_channel_name_input
            .update(cx, |input, cx| input.clear(cx));
        self.create_channel_desc_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.quick_switcher.is_some() {
                s.close_quick_switcher();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_create_channel_modal();
            cx.notify();
        });
        window.focus(&self.create_channel_name_input.focus_handle(cx));
    }

    pub(in crate::view) fn toggle_create_channel_is_private(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            if let Some(m) = &mut s.create_channel_modal {
                m.is_private = !m.is_private;
                m.channel_id = Some(uuid::Uuid::new_v4().to_string());
            }
            cx.notify();
        });
    }

    pub(in crate::view) fn render_create_channel_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let modal = state.create_channel_modal.as_ref().unwrap();
        let in_flight = modal.in_flight;
        let error = modal.error.clone();
        let is_private = modal.is_private;
        let desc_count = modal.description.trim().chars().count();
        let desc_too_long = desc_count > 250;
        let counter_color = if desc_too_long {
            theme.danger
        } else {
            theme.text_muted
        };
        let is_private_focused = self.create_channel_private_focus.is_focused(window);

        div()
            .w(px(400.0))
            .p_5()
            .rounded_lg()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_3()
            .on_action(cx.listener(|this, _: &crate::input::Tab, window, cx| {
                if this
                    .create_channel_name_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_desc_input.focus_handle(cx));
                } else if this
                    .create_channel_desc_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_private_focus);
                } else if this.create_channel_private_focus.is_focused(window) {
                    window.focus(&this.create_channel_name_input.focus_handle(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &crate::input::TabPrev, window, cx| {
                if this
                    .create_channel_name_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_private_focus);
                } else if this
                    .create_channel_desc_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_name_input.focus_handle(cx));
                } else if this.create_channel_private_focus.is_focused(window) {
                    window.focus(&this.create_channel_desc_input.focus_handle(cx));
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("新規チャンネル作成"),
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
                                        s.close_create_channel_modal();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
            .when_some(error, |d, err| {
                d.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .text_xs()
                        .text_color(theme.danger)
                        .child(err),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("チャンネル名"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.border)
                            .child(self.create_channel_name_input.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("説明 (任意)"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(counter_color)
                                    .child(format!("{desc_count}/250")),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(if desc_too_long {
                                theme.danger
                            } else {
                                theme.border
                            })
                            .child(self.create_channel_desc_input.clone()),
                    ),
            )
            .child(
                div()
                    .id("create-channel-private-toggle")
                    .track_focus(&self.create_channel_private_focus)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_1()
                    .rounded_md()
                    .when(is_private_focused, |d| {
                        d.border_1().border_color(theme.accent).bg(theme.bg_surface)
                    })
                    .hover(|s| s.cursor_pointer().bg(theme.bg_hover))
                    .child(
                        div()
                            .w_4()
                            .h_4()
                            .rounded_sm()
                            .border_1()
                            .border_color(if is_private {
                                theme.border_accent
                            } else {
                                theme.border
                            })
                            .bg(if is_private {
                                theme.accent_button
                            } else {
                                theme.bg_surface
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(theme.text_on_accent)
                            .child(if is_private { "✓" } else { "" }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_primary)
                            .child("非公開チャンネルにする"),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.toggle_create_channel_is_private(cx);
                        }),
                    )
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if ev.keystroke.key == "tab" {
                            if ev.keystroke.modifiers.shift {
                                window.focus(&this.create_channel_desc_input.focus_handle(cx));
                            } else {
                                window.focus(&this.create_channel_name_input.focus_handle(cx));
                            }
                        } else if is_activate_key(&ev.keystroke.key) {
                            this.toggle_create_channel_is_private(cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .mt_2()
                    .child(
                        div()
                            .px_3()
                            .py_1p5()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("キャンセル")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.close_create_channel_modal();
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_1p5()
                            .rounded_md()
                            .bg(theme.accent_button)
                            .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_on_accent)
                            .child(if in_flight { "作成中…" } else { "作成" })
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.submit_create_channel();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
    }
}
