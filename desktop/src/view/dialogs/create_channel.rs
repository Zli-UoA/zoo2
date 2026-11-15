//! チャンネル作成ダイアログ (`CreateChannelDialog`)。

use gpui::{
    Context, Div, FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    Styled, Window, div, prelude::*, px,
};

use super::{checkbox_box, cycle_focus};
use crate::fonts;
use crate::ui::button::{Button, ButtonVariant};
use crate::ui::dialog::Dialog;
use crate::ui::input::Input;
use crate::ui::textarea::Textarea;
use crate::ui::{TextSpec, h_flex, palette, v_flex};
use crate::view::{WorkspaceView, is_activate_key};

/// 説明の最大文字数 (`MAX_CHANNEL_DESCRIPTION_LENGTH`)。
const MAX_DESCRIPTION: usize = 250;

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

    /// Tab で回る順 (名前、説明、非公開、取り消し、作成)。
    /// 作成ボタンが無効なあいだは、`Button` が tab stop にしないので、巡回からも外す。
    fn create_channel_focus_order(
        &self,
        submit_enabled: bool,
        cx: &Context<Self>,
    ) -> Vec<FocusHandle> {
        let mut order = vec![
            self.create_channel_name_input.focus_handle(cx),
            self.create_channel_desc_input.focus_handle(cx),
            self.create_channel_private_focus.clone(),
            self.create_cancel_focus.clone(),
        ];
        if submit_enabled {
            order.push(self.create_submit_focus.clone());
        }
        order
    }

    pub(in crate::view) fn render_create_channel_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        let xs = TextSpec::XS;
        let state = self.state.read(cx);
        let modal = state.create_channel_modal.as_ref().unwrap();
        let in_flight = modal.in_flight;
        let error = modal.error.clone();
        let is_private = modal.is_private;
        let name_empty = modal.channel_name.trim().is_empty();
        let desc_count = modal.description.trim().chars().count();
        let desc_too_long = desc_count > MAX_DESCRIPTION;
        let is_private_focused = self.create_channel_private_focus.is_focused(window);
        let submit_disabled = in_flight || name_empty || desc_too_long;
        let order = self.create_channel_focus_order(!submit_disabled, cx);
        let order_prev = order.clone();
        let order_key = order.clone();
        let max = MAX_DESCRIPTION.to_string();
        let length = desc_count.to_string();
        // 枠は FocusHandle を持たず、中の入力欄にフォーカスがあるかで見た目を切り替える。
        let name_focused = self
            .create_channel_name_input
            .focus_handle(cx)
            .is_focused(window);
        let desc_focused = self
            .create_channel_desc_input
            .focus_handle(cx)
            .is_focused(window);

        let counter = i18n.t_with(
            "channels.descriptionCounter",
            &[("length", &length), ("max", &max)],
        );
        let help = if desc_too_long {
            i18n.t_with(
                "channels.descriptionTooLong",
                &[("length", &length), ("max", &max)],
            )
        } else {
            i18n.t_with("channels.descriptionHelp", &[("max", &max)])
        };

        // `<form className="flex flex-col gap-3">`
        let form = v_flex()
            .gap(px(12.0))
            .child(
                Input::new()
                    .focused(name_focused)
                    .child(self.create_channel_name_input.clone()),
            )
            .child(
                // `flex flex-col gap-1.5`
                v_flex()
                    .gap(px(6.0))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                // `text-sm font-medium`
                                div()
                                    .font(fonts::sans(500.0))
                                    .child(i18n.t("channels.descriptionLabel")),
                            )
                            .child(
                                div()
                                    .text_size(xs.size())
                                    .line_height(xs.line())
                                    .text_color(if desc_too_long {
                                        t.destructive
                                    } else {
                                        t.muted_foreground
                                    })
                                    .child(counter),
                            ),
                    )
                    .child(
                        Textarea::new()
                            .invalid(desc_too_long)
                            .focused(desc_focused)
                            .child(self.create_channel_desc_input.clone()),
                    )
                    .child(
                        div()
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .text_color(if desc_too_long {
                                t.destructive
                            } else {
                                t.muted_foreground
                            })
                            .child(help),
                    ),
            )
            .child(
                // `<label className="flex items-center gap-2 text-sm">`
                h_flex()
                    .id("create-channel-private-toggle")
                    .track_focus(&self.create_channel_private_focus)
                    .gap(px(8.0))
                    .hover(|s| s.cursor_pointer())
                    .child(checkbox_box(&p, is_private, is_private_focused, false))
                    .child(i18n.t("channels.makePrivate"))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.toggle_create_channel_is_private(cx);
                        }),
                    )
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                        if is_activate_key(&ev.keystroke.key) {
                            this.toggle_create_channel_is_private(cx);
                        }
                    })),
            )
            .when_some(error, |d, err| {
                d.child(
                    div()
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.destructive)
                        .child(i18n.t_with("common.writeErrorDetail", &[("error", &err)])),
                )
            });

        let submit_label = if in_flight {
            i18n.t("channels.creating")
        } else {
            i18n.t("channels.create")
        };
        let dialog = Dialog::new("create-channel", i18n.t("channels.createTitle"))
            .description(i18n.t("channels.visibilityHelp"))
            .on_close(cx.listener(|this, _, _, cx| {
                this.state.update(cx, |s, cx| {
                    s.close_create_channel_modal();
                    cx.notify();
                });
            }))
            .child(form)
            .footer(
                Button::new("create-channel-cancel")
                    .focus_handle(&self.create_cancel_focus)
                    .variant(ButtonVariant::Outline)
                    .label(i18n.t("common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.close_create_channel_modal();
                            cx.notify();
                        });
                    })),
            )
            .footer(
                Button::new("create-channel-submit")
                    .focus_handle(&self.create_submit_focus)
                    .label(submit_label)
                    .disabled(submit_disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.submit_create_channel();
                            cx.notify();
                        });
                    })),
            );

        div()
            .text_size(sm.size())
            .on_action(cx.listener(move |_, _: &crate::input::Tab, window, _| {
                cycle_focus(window, &order, false);
            }))
            .on_action(cx.listener(move |_, _: &crate::input::TabPrev, window, _| {
                cycle_focus(window, &order_prev, true);
            }))
            // 入力欄の外 (チェックボックスとボタン) の Tab。入力欄の Tab は上の action が受ける。
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                // ボタンやチェックボックスにフォーカスがあっても Esc で閉じる (削除確認と同じ方式)。
                // 閉じる処理と、閉じたあとのフォーカスの戻し先は、`app_shell.rs` の Esc と同じ。
                if ev.keystroke.key == "escape" {
                    this.create_channel_name_input
                        .update(cx, |i, cx| i.clear(cx));
                    this.create_channel_desc_input
                        .update(cx, |i, cx| i.clear(cx));
                    this.state.update(cx, |s, cx| {
                        s.close_create_channel_modal();
                        cx.notify();
                    });
                    window.focus(&this.composer_input.focus_handle(cx));
                    // 上のルートの Esc 処理 (スレッドを閉じるなど) まで届かせない。
                    cx.stop_propagation();
                    return;
                }
                if ev.keystroke.key == "tab"
                    && !this
                        .create_channel_name_input
                        .focus_handle(cx)
                        .is_focused(window)
                    && !this
                        .create_channel_desc_input
                        .focus_handle(cx)
                        .is_focused(window)
                {
                    cycle_focus(window, &order_key, ev.keystroke.modifiers.shift);
                    cx.stop_propagation();
                }
            }))
            .child(dialog)
    }
}
