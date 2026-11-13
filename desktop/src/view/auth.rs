//! ログイン画面 (frontend の `components/auth/**` に対応)。

use gpui::{
    ClipboardItem, Context, Div, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent,
    ParentElement, Styled, Window, div, prelude::*, px,
};
use zoo2_client_core::config::AuthConfig;

use crate::login::{self, LoginPhase};
use crate::view::{WorkspaceView, is_activate_key};

impl WorkspaceView {
    pub(crate) fn render_login_view(
        &self,
        phase: &LoginPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let api_url = state.api_base_url.clone();
        let auth_summary = state.auth_summary.clone();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));

        if !self.login_primary_focus.is_focused(window)
            && !self.login_secondary_focus.is_focused(window)
            && !self.login_cancel_focus.is_focused(window)
            && !matches!(phase, LoginPhase::Checking)
        {
            window.focus(&self.login_primary_focus);
        }

        let body = match phase {
            LoginPhase::Checking => div().flex().flex_col().items_center().gap_3().child(
                div()
                    .text_base()
                    .text_color(theme.accent)
                    .child("接続とセッションを確認しています…"),
            ),
            LoginPhase::Ready { error } => {
                let mut col = div().flex().flex_col().gap_3().w_full();
                if let Some(err) = error {
                    col = col.child(
                        div()
                            .w_full()
                            .p_3()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.danger)
                            .text_sm()
                            .text_color(theme.warning)
                            .child(err.clone()),
                    );
                }
                if is_oidc {
                    let primary_focused = self.login_primary_focus.is_focused(window);
                    let secondary_focused = self.login_secondary_focus.is_focused(window);
                    col = col
                        .child(
                            div()
                                .id("btn-start-device-login")
                                .track_focus(&self.login_primary_focus)
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme.accent_button)
                                .border_1()
                                .border_color(if primary_focused {
                                    theme.text_primary
                                } else {
                                    theme.accent_button
                                })
                                .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_on_accent)
                                .child("ブラウザでログイン (Enter)")
                                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                    if is_activate_key(&ev.keystroke.key) {
                                        this.state.update(cx, |s, cx| {
                                            s.start_device_login();
                                            cx.notify();
                                        });
                                    }
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.start_device_login();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .id("btn-retry-initial-auth")
                                .track_focus(&self.login_secondary_focus)
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme.bg_surface)
                                .border_1()
                                .border_color(if secondary_focused {
                                    theme.border_accent
                                } else {
                                    theme.border
                                })
                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child("保存済みトークンで再試行")
                                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                    if is_activate_key(&ev.keystroke.key) {
                                        this.state.update(cx, |s, cx| {
                                            s.start_initial_auth();
                                            cx.notify();
                                        });
                                    }
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.start_initial_auth();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        );
                } else {
                    let primary_focused = self.login_primary_focus.is_focused(window);
                    col = col.child(
                        div()
                            .id("btn-retry-dev-auth")
                            .track_focus(&self.login_primary_focus)
                            .w_full()
                            .py_2()
                            .px_4()
                            .rounded_md()
                            .bg(theme.accent_button)
                            .border_1()
                            .border_color(if primary_focused {
                                theme.text_primary
                            } else {
                                theme.accent_button
                            })
                            .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_on_accent)
                            .child("再接続する (Enter)")
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                if is_activate_key(&ev.keystroke.key) {
                                    this.state.update(cx, |s, cx| {
                                        s.start_initial_auth();
                                        cx.notify();
                                    });
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.start_initial_auth();
                                        cx.notify();
                                    });
                                }),
                            ),
                    );
                }
                col
            }
            LoginPhase::AwaitingApproval {
                verification_uri,
                user_code,
            } => {
                let uri_for_open = verification_uri.clone();
                let uri_for_key = verification_uri.clone();
                let code_for_copy = user_code.clone();
                let code_for_key = user_code.clone();
                let primary_focused = self.login_primary_focus.is_focused(window);
                let secondary_focused = self.login_secondary_focus.is_focused(window);
                let cancel_focused = self.login_cancel_focus.is_focused(window);
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_primary)
                            .child("ブラウザで次のコードを確認し、ログインを承認してください。"),
                    )
                    .child(
                        div()
                            .w_full()
                            .p_4()
                            .rounded_md()
                            .bg(theme.bg_primary)
                            .border_1()
                            .border_color(theme.border_accent)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("確認コード"),
                            )
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.accent)
                                    .child(user_code.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(verification_uri.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .w_full()
                            .child(
                                div()
                                    .id("btn-open-browser")
                                    .track_focus(&self.login_primary_focus)
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme.accent_button)
                                    .border_1()
                                    .border_color(if primary_focused {
                                        theme.text_primary
                                    } else {
                                        theme.accent_button
                                    })
                                    .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_on_accent)
                                    .child("ブラウザを開く")
                                    .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, _| {
                                        if is_activate_key(&ev.keystroke.key) {
                                            login::open_browser(&uri_for_key);
                                        }
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |_, _: &MouseUpEvent, _, _| {
                                            login::open_browser(&uri_for_open);
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("btn-copy-user-code")
                                    .track_focus(&self.login_secondary_focus)
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(if secondary_focused {
                                        theme.border_accent
                                    } else {
                                        theme.border
                                    })
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .text_color(theme.text_primary)
                                    .child("コードをコピー")
                                    .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                                        if is_activate_key(&ev.keystroke.key) {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                code_for_key.clone(),
                                            ));
                                        }
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                code_for_copy.clone(),
                                            ));
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("btn-cancel-device-login")
                            .track_focus(&self.login_cancel_focus)
                            .w_full()
                            .py_1p5()
                            .px_3()
                            .rounded_md()
                            .border_1()
                            .border_color(if cancel_focused {
                                theme.border_accent
                            } else {
                                theme.bg_secondary
                            })
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_center()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("キャンセル (Esc)")
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                if is_activate_key(&ev.keystroke.key)
                                    || ev.keystroke.key == "escape"
                                {
                                    this.state.update(cx, |s, cx| {
                                        s.cancel_device_login();
                                        cx.notify();
                                    });
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.cancel_device_login();
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
            }
        };

        let is_awaiting = matches!(phase, LoginPhase::AwaitingApproval { .. });
        let is_ready_oidc = matches!(phase, LoginPhase::Ready { .. }) && is_oidc;

        div()
            .flex()
            .size_full()
            .bg(theme.bg_primary)
            .text_color(theme.text_primary)
            .justify_center()
            .items_center()
            .p_6()
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                if key == "escape" && is_awaiting {
                    this.state.update(cx, |s, cx| {
                        s.cancel_device_login();
                        cx.notify();
                    });
                    return;
                }
                let forward =
                    matches!(key, "tab" | "down" | "right") && !ev.keystroke.modifiers.shift;
                let backward =
                    (key == "tab" && ev.keystroke.modifiers.shift) || matches!(key, "up" | "left");
                if !forward && !backward {
                    return;
                }
                if is_awaiting {
                    let handles = [
                        &this.login_primary_focus,
                        &this.login_secondary_focus,
                        &this.login_cancel_focus,
                    ];
                    let current = handles
                        .iter()
                        .position(|h| h.is_focused(window))
                        .unwrap_or(0);
                    let next = if forward {
                        (current + 1) % handles.len()
                    } else {
                        (current + handles.len() - 1) % handles.len()
                    };
                    window.focus(handles[next]);
                    cx.notify();
                } else if is_ready_oidc {
                    let handles = [&this.login_primary_focus, &this.login_secondary_focus];
                    let current = handles
                        .iter()
                        .position(|h| h.is_focused(window))
                        .unwrap_or(0);
                    let next = (current + 1) % handles.len();
                    window.focus(handles[next]);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .w(px(460.0))
                    .p_6()
                    .rounded_lg()
                    .bg(theme.bg_secondary)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child("zoo2 Desktop"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(format!("接続先: {api_url}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(format!("認証方式: {auth_summary}")),
                            ),
                    )
                    .child(body),
            )
    }
}
