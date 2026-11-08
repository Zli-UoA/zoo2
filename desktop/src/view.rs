//! GPUI による画面描画 (`WorkspaceView`、`LoginView`、`SidebarView`、`TimelineView`、`ComposerView`、`ThreadView`)。

use gpui::{
    ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};
use jiff::{Zoned, civil::DateTime, tz::TimeZone};
use zoo2_client_core::{
    config::AuthConfig,
    electric::{ShapeId, rows::Message},
    store::ReactionGroup,
};

use crate::{
    app::{AppState, QUICK_EMOJIS, Target, reaction_label},
    input::{InputEvent, TextInput},
    login::{self, LoginPhase},
    mfm_view, theme,
};

pub struct WorkspaceView {
    pub state: Entity<AppState>,
    pub search_input: Entity<TextInput>,
    pub composer_input: Entity<TextInput>,
    pub reaction_input: Entity<TextInput>,
    pub timeline_scroll: ScrollHandle,
    pub thread_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    login_primary_focus: FocusHandle,
    login_secondary_focus: FocusHandle,
    login_cancel_focus: FocusHandle,
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl WorkspaceView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| TextInput::new_single_line(cx, "チャンネル・DM を検索…"));
        let composer_input = cx.new(|cx| {
            TextInput::new_multiline(cx, "メッセージを入力 (Enter で送信、Shift+Enter で改行)…")
        });
        let reaction_input = cx.new(|cx| {
            TextInput::new_single_line(cx, "絵文字またはショートコード (例: thumbsup, 🎉)")
        });

        cx.subscribe(
            &search_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        state.search_query = text.clone();
                        cx.notify();
                    });
                }
                InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        if let Some(first) = state.filtered_channels().first().map(|c| c.id.clone())
                        {
                            state.open_channel(first);
                            cx.notify();
                        }
                    });
                }
                InputEvent::Cancel => {
                    this.search_input.update(cx, |input, cx| input.clear(cx));
                    this.state.update(cx, |state, cx| {
                        state.search_query.clear();
                        cx.notify();
                    });
                }
            },
        )
        .detach();

        cx.subscribe(
            &composer_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        state.on_composer_changed(text.clone());
                        cx.notify();
                    });
                }
                InputEvent::Submit(text) => {
                    let submitted = this.state.update(cx, |state, cx| {
                        let ok = state.submit_message(text.clone());
                        cx.notify();
                        ok
                    });
                    if submitted {
                        this.composer_input.update(cx, |input, cx| input.clear(cx));
                    }
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        state.cancel_target();
                        cx.notify();
                    });
                }
            },
        )
        .detach();

        cx.subscribe(
            &reaction_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(_) => {}
                InputEvent::Submit(text) => {
                    let ok = this.state.update(cx, |state, cx| {
                        let Some((_, msg_id)) = state.active_reaction_message.clone() else {
                            return false;
                        };
                        let res = state.submit_reaction_input(&msg_id, text);
                        cx.notify();
                        res
                    });
                    if ok {
                        this.reaction_input.update(cx, |input, cx| input.clear(cx));
                    }
                }
                InputEvent::Cancel => {
                    this.reaction_input.update(cx, |input, cx| input.clear(cx));
                    this.state.update(cx, |state, cx| {
                        state.active_reaction_message = None;
                        cx.notify();
                    });
                }
            },
        )
        .detach();

        cx.observe(&state, |this, state, cx| {
            let (pending_composer, pending_search) = state.update(cx, |s, _| {
                (s.pending_composer_text.take(), s.pending_search_text.take())
            });
            if let Some(text) = pending_composer {
                this.composer_input
                    .update(cx, |input, cx| input.set_text(text, cx));
            }
            if let Some(text) = pending_search {
                this.search_input
                    .update(cx, |input, cx| input.set_text(text, cx));
            }
            cx.notify();
        })
        .detach();

        Self {
            state,
            search_input,
            composer_input,
            reaction_input,
            timeline_scroll: ScrollHandle::new(),
            thread_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            login_primary_focus: cx.focus_handle(),
            login_secondary_focus: cx.focus_handle(),
            login_cancel_focus: cx.focus_handle(),
        }
    }

    fn submit_composer(&mut self, cx: &mut Context<Self>) {
        self.composer_input
            .update(cx, |input, cx| input.request_submit(cx));
    }

    fn render_login_view(
        &self,
        phase: &LoginPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
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
                    .text_color(theme::ACCENT)
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
                            .bg(theme::BG_SURFACE)
                            .border_1()
                            .border_color(theme::DANGER)
                            .text_sm()
                            .text_color(theme::WARNING)
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
                                .bg(theme::ACCENT_BUTTON)
                                .border_1()
                                .border_color(if primary_focused {
                                    theme::TEXT_PRIMARY
                                } else {
                                    theme::ACCENT_BUTTON
                                })
                                .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::TEXT_PRIMARY)
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
                                .bg(theme::BG_SURFACE)
                                .border_1()
                                .border_color(if secondary_focused {
                                    theme::BORDER_ACCENT
                                } else {
                                    theme::BORDER
                                })
                                .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .text_color(theme::TEXT_MUTED)
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
                            .bg(theme::ACCENT_BUTTON)
                            .border_1()
                            .border_color(if primary_focused {
                                theme::TEXT_PRIMARY
                            } else {
                                theme::ACCENT_BUTTON
                            })
                            .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT_PRIMARY)
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
                            .text_color(theme::TEXT_PRIMARY)
                            .child("ブラウザで次のコードを確認し、ログインを承認してください。"),
                    )
                    .child(
                        div()
                            .w_full()
                            .p_4()
                            .rounded_md()
                            .bg(theme::BG_PRIMARY)
                            .border_1()
                            .border_color(theme::BORDER_ACCENT)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("確認コード"),
                            )
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::ACCENT)
                                    .child(user_code.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
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
                                    .bg(theme::ACCENT_BUTTON)
                                    .border_1()
                                    .border_color(if primary_focused {
                                        theme::TEXT_PRIMARY
                                    } else {
                                        theme::ACCENT_BUTTON
                                    })
                                    .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
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
                                    .bg(theme::BG_SURFACE)
                                    .border_1()
                                    .border_color(if secondary_focused {
                                        theme::BORDER_ACCENT
                                    } else {
                                        theme::BORDER
                                    })
                                    .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .text_color(theme::TEXT_PRIMARY)
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
                                theme::BORDER_ACCENT
                            } else {
                                theme::BG_SECONDARY
                            })
                            .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                            .text_center()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
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
            .bg(theme::BG_PRIMARY)
            .text_color(theme::TEXT_PRIMARY)
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
                    .bg(theme::BG_SECONDARY)
                    .border_1()
                    .border_color(theme::BORDER)
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
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child("zoo2 Desktop"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(format!("接続先: {api_url}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(format!("認証方式: {auth_summary}")),
                            ),
                    )
                    .child(body),
            )
    }

    fn render_sidebar_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_summary = state
            .me
            .as_ref()
            .map(|m| format!("{} (@{})", m.display_name, m.user_name))
            .unwrap_or_default();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));
        let channels_synced = state.store.is_synced(&ShapeId::Channels);
        let sync_error = state.sync_error();
        let notice = state.notice.as_ref().map(|(m, _)| m.clone());
        let current_channel = state.current_channel.clone();
        let unread = state.unread.clone();

        let filtered = state.filtered_channels();
        let mut normal_rows = Vec::new();
        let mut dm_rows = Vec::new();

        for ch in filtered {
            let ch_id = ch.id.clone();
            let ch_id_click = ch_id.clone();
            let is_dm = ch.is_dm;
            let is_open = current_channel.as_deref() == Some(ch_id.as_str());
            let is_unread = unread.contains(&ch_id);
            let is_member = state.store.is_member(&ch_id, &me_id);
            let mut label = state.store.channel_label(ch, &me_id);
            if ch.is_private && !ch.is_dm {
                label.push_str(" 🔒");
            }
            if ch.archive_at.is_some() {
                label.push_str(" (アーカイブ)");
            }

            let row_bg = if is_open {
                theme::BG_SELECTED
            } else {
                theme::BG_SECONDARY
            };
            let text_color = if is_open || is_unread {
                theme::TEXT_PRIMARY
            } else if is_member {
                theme::TEXT_MUTED
            } else {
                theme::TEXT_DIM
            };
            let weight = if is_open || is_unread {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };

            let row = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "sidebar-ch-{ch_id}"
                ))))
                .w_full()
                .px_2p5()
                .py_1p5()
                .rounded_md()
                .bg(row_bg)
                .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let id = ch_id_click.clone();
                        this.state.update(cx, |s, cx| {
                            s.open_channel(id);
                            cx.notify();
                        });
                    }),
                )
                .child(
                    div()
                        .w(px(10.0))
                        .text_xs()
                        .text_color(theme::ACCENT)
                        .child(if is_unread { "●" } else { "" }),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_sm()
                        .font_weight(weight)
                        .text_color(text_color)
                        .child(label),
                );

            if is_dm {
                dm_rows.push(row);
            } else {
                normal_rows.push(row);
            }
        }

        div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .h_full()
            .bg(theme::BG_SECONDARY)
            .border_r_1()
            .border_color(theme::BORDER)
            // サイドバーヘッダー
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(theme::BORDER)
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
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child("zoo2"),
                            )
                            .when(is_oidc, |d| {
                                d.child(
                                    div()
                                        .id("btn-logout")
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SURFACE)
                                        .border_1()
                                        .border_color(theme::BORDER)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("ログアウト")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.logout();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(me_summary),
                    )
                    .child(self.search_input.clone()),
            )
            // チャンネル・DM 一覧
            .child(
                div()
                    .id("sidebar-scroll-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT_MUTED)
                            .child(if channels_synced {
                                "チャンネル"
                            } else if sync_error.is_some() {
                                "チャンネル (同期停止)"
                            } else {
                                "チャンネル (同期中…)"
                            }),
                    )
                    .when_some(sync_error, |d, err| {
                        d.child(
                            div()
                                .mx_2()
                                .my_1()
                                .p_2()
                                .rounded_md()
                                .bg(theme::BG_SURFACE)
                                .border_1()
                                .border_color(theme::DANGER)
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(div().text_xs().text_color(theme::WARNING).child(err))
                                .child(
                                    div()
                                        .id("btn-sidebar-retry-sync")
                                        .px_2()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(theme::ACCENT_BUTTON)
                                        .hover(|s| {
                                            s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer()
                                        })
                                        .text_center()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme::TEXT_PRIMARY)
                                        .child("再同期する")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.retry_sync();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                ),
                        )
                    })
                    .when(current_channel.is_none(), |d| {
                        d.when_some(notice, |d, msg| {
                            d.child(
                                div()
                                    .mx_2()
                                    .my_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(theme::BG_SURFACE)
                                    .border_1()
                                    .border_color(theme::WARNING)
                                    .text_xs()
                                    .text_color(theme::WARNING)
                                    .child(msg),
                            )
                        })
                    })
                    .children(normal_rows)
                    .when(!dm_rows.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_2()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::TEXT_MUTED)
                                .child("ダイレクトメッセージ"),
                        )
                        .children(dm_rows)
                    }),
            )
    }

    fn render_timeline_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let Some(channel_id) = state.current_channel.clone() else {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .justify_center()
                .items_center()
                .p_6()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::TEXT_PRIMARY)
                        .child("チャンネルを選択してください"),
                )
                .child(
                    div().text_sm().text_color(theme::TEXT_MUTED).child(
                        "左のサイドバーからチャンネルまたはダイレクトメッセージを開けます。",
                    ),
                );
        };

        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_user_name = state
            .me
            .as_ref()
            .map(|m| m.user_name.clone())
            .unwrap_or_default();
        let channel = state.store.channel(&channel_id);
        let ch_label = channel
            .map(|c| state.store.channel_label(c, &me_id))
            .unwrap_or_else(|| format!("#{channel_id}"));
        let ch_desc = channel
            .and_then(|c| c.description.clone())
            .filter(|d| !d.is_empty());
        let is_archived = channel.is_some_and(|c| c.archive_at.is_some());
        let is_member = state.store.is_member(&channel_id, &me_id);
        let can_join = channel.is_some_and(|c| state.can_join_channel(c));
        let can_leave = channel.is_some_and(|c| state.can_leave_channel(c));
        let failed_draft_count = state.failed_drafts.len();
        let confirm_delete_id = state.confirm_delete_message.clone();
        let synced = state
            .store
            .is_synced(&ShapeId::Messages(channel_id.clone()));
        let timeline_at_bottom = state.timeline_at_bottom;

        let all_root_messages = state.channel_cache.root_messages.clone();
        let (start_idx, end_idx) = state.visible_timeline_bounds(&all_root_messages);
        let hidden_before = start_idx;
        let hidden_after = all_root_messages.len().saturating_sub(end_idx);
        let visible_roots = &all_root_messages[start_idx..end_idx];
        let reply_counts = state.channel_cache.reply_counts.clone();
        let reactions_by_msg = state.channel_cache.reactions_by_message.clone();

        let mut timeline_items: Vec<Div> = Vec::new();
        if hidden_before > 0 {
            timeline_items.push(
                div().w_full().flex().justify_center().py_1().child(
                    div()
                        .id("btn-load-older-messages")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme::BG_SURFACE)
                        .border_1()
                        .border_color(theme::BORDER)
                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                        .text_xs()
                        .text_color(theme::ACCENT)
                        .child(format!("過去のメッセージを表示 (残り {hidden_before} 件)"))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                this.state.update(cx, |s, cx| {
                                    s.load_older_messages();
                                    cx.notify();
                                });
                            }),
                        ),
                ),
            );
        }
        let mut last_date = None;
        for msg in visible_roots {
            let date = local_zoned(msg.created_at).map(|z| z.date());
            if date.is_some() && date != last_date {
                if let Some(d) = date {
                    timeline_items.push(render_date_separator(&d.to_string()));
                }
                last_date = date;
            }
            let reply_count = reply_counts.get(&msg.id).copied().unwrap_or(0);
            let reactions = reactions_by_msg.get(&msg.id).cloned().unwrap_or_default();
            timeline_items.push(self.render_message_card(
                msg,
                &me_id,
                &me_user_name,
                is_archived,
                reactions,
                Some(reply_count),
                "timeline",
                cx,
            ));
        }

        let ch_id_for_join = channel_id.clone();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            // チャンネルヘッダー
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(theme::BG_SURFACE)
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child(ch_label),
                            )
                            .when_some(ch_desc, |d, desc| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(format!("— {desc}")),
                                )
                            })
                            .when(is_archived, |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .text_xs()
                                        .text_color(theme::WARNING)
                                        .child("アーカイブ済み"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(failed_draft_count > 0, |d| {
                                d.child(
                                    div()
                                        .id("btn-restore-draft")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme::BG_SECONDARY)
                                        .border_1()
                                        .border_color(theme::WARNING)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::WARNING)
                                        .child(format!("下書きを復元 ({failed_draft_count})"))
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.restore_failed_draft();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            })
                            .when(can_join || can_leave, |d| {
                                d.child(
                                    div()
                                        .id("btn-join-or-leave")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(if is_member {
                                            theme::BG_SECONDARY
                                        } else {
                                            theme::ACCENT_BUTTON
                                        })
                                        .border_1()
                                        .border_color(theme::BORDER)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme::TEXT_PRIMARY)
                                        .child(if is_member {
                                            "退出する"
                                        } else {
                                            "参加する"
                                        })
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                                let ch = ch_id_for_join.clone();
                                                this.state.update(cx, |s, cx| {
                                                    s.join_or_leave(ch, !is_member);
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            }),
                    ),
            )
            // 削除確認バー
            .when(confirm_delete_id.is_some(), |d| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(theme::BG_SECONDARY)
                        .border_b_1()
                        .border_color(theme::DANGER)
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::WARNING)
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
                                        .bg(theme::DANGER)
                                        .hover(|s| s.cursor_pointer())
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme::TEXT_PRIMARY)
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
                                        .bg(theme::BG_SURFACE)
                                        .border_1()
                                        .border_color(theme::BORDER)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
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
                        ),
                )
            })
            // タイムライン本体
            .child(
                div()
                    .id("timeline-scroll-area")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.timeline_scroll)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(!synced, |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme::TEXT_MUTED)
                                .child("メッセージを読み込み中…"),
                        )
                    })
                    .when(synced && timeline_items.is_empty(), |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme::TEXT_MUTED)
                                .child("まだメッセージがありません"),
                        )
                    })
                    .children(timeline_items),
            )
            // 最新へ戻るバー
            .when(!timeline_at_bottom || hidden_after > 0, |d| {
                d.child(
                    div()
                        .px_4()
                        .py_1p5()
                        .bg(theme::BG_SURFACE)
                        .border_t_1()
                        .border_color(theme::BORDER)
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("btn-jump-to-latest")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(theme::ACCENT_BUTTON)
                                .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::TEXT_PRIMARY)
                                .child(if hidden_after > 0 {
                                    format!("↓ 最新メッセージへ移動 (新着ほか {hidden_after} 件)")
                                } else {
                                    "↓ 最新メッセージへ移動".to_string()
                                })
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.jump_to_latest_messages();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        ),
                )
            })
            // 下部入力欄
            .child(self.render_composer_view(cx))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_message_card(
        &self,
        message: &Message,
        me_id: &str,
        me_user_name: &str,
        is_archived: bool,
        reactions: Vec<ReactionGroup>,
        reply_count: Option<usize>,
        pane_prefix: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let msg_id = message.id.clone();
        let author_name = state.store.display_name(&message.user_id);
        let is_bot = state.store.user(&message.user_id).is_some_and(|u| u.is_bot);
        let is_mine = message.user_id == me_id;
        let is_edited = message.updated_at.is_some();
        let time_str = local_time_string(message.created_at);
        let reaction_picker_open = !is_archived
            && state
                .active_reaction_message
                .as_ref()
                .is_some_and(|(pane, id)| pane == pane_prefix && id == &msg_id);

        let msg_id_reply = msg_id.clone();
        let msg_id_react = msg_id.clone();
        let pane_for_react = pane_prefix.to_string();
        let msg_id_edit = msg_id.clone();
        let msg_id_delete = msg_id.clone();
        let msg_id_thread = msg_id.clone();

        let mut reaction_pills = Vec::new();
        for group in reactions {
            let emoji = group.emoji.clone();
            let msg_for_toggle = msg_id.clone();
            let pill_bg = if group.mine {
                theme::BG_REACTION_MINE
            } else {
                theme::BG_SECONDARY
            };
            let pill_border = if group.mine {
                theme::BORDER_ACCENT
            } else {
                theme::BORDER
            };
            let pill_text = if group.mine {
                theme::ACCENT
            } else {
                theme::TEXT_MUTED
            };
            let pill = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "{pane_prefix}-react-pill-{}-{emoji}",
                    message.id
                ))))
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(pill_bg)
                .border_1()
                .border_color(pill_border)
                .text_xs()
                .text_color(pill_text)
                .child(format!("{} {}", reaction_label(&group.emoji), group.count))
                .when(!is_archived, |d| {
                    d.hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let m = msg_for_toggle.clone();
                                let e = emoji.clone();
                                this.state.update(cx, |s, cx| {
                                    s.toggle_reaction(&m, &e);
                                    cx.notify();
                                });
                            }),
                        )
                });
            reaction_pills.push(pill);
        }

        let mut quick_emoji_buttons = Vec::new();
        if reaction_picker_open {
            for &emoji in QUICK_EMOJIS {
                let m_id = msg_id.clone();
                let emoji_str = emoji.to_string();
                quick_emoji_buttons.push(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "{pane_prefix}-quick-emoji-{msg_id}-{emoji}"
                        ))))
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme::BG_PRIMARY)
                        .border_1()
                        .border_color(theme::BORDER)
                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                        .text_sm()
                        .child(emoji)
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let m = m_id.clone();
                                let e = emoji_str.clone();
                                this.state.update(cx, |s, cx| {
                                    s.active_reaction_message = None;
                                    s.toggle_reaction(&m, &e);
                                    cx.notify();
                                });
                            }),
                        ),
                );
            }
        }

        let root_for_reply_btn = message
            .parent_id
            .clone()
            .unwrap_or_else(|| msg_id_reply.clone());

        div()
            .w_full()
            .p_3()
            .rounded_md()
            .bg(theme::BG_SURFACE)
            .border_1()
            .border_color(theme::BORDER)
            .flex()
            .flex_col()
            .gap_1p5()
            // メッセージヘッダー (投稿者・時刻・操作ボタン)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .gap_2()
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
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child(author_name),
                            )
                            .when(is_bot, |d| {
                                d.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("BOT"),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child(time_str),
                            )
                            .when(is_edited, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::TEXT_DIM)
                                        .child("(編集済み)"),
                                )
                            }),
                    )
                    // 操作ボタン群
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .when(!is_archived, |d| {
                                d.child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-reply-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("返信")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let r_id = root_for_reply_btn.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.open_thread(r_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-react-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("+ リアクション")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let m_id = msg_id_react.clone();
                                                    let pane_id = pane_for_react.clone();
                                                    let opened = this.state.update(cx, |s, cx| {
                                                        let target_key = (pane_id, m_id);
                                                        if s.active_reaction_message.as_ref()
                                                            == Some(&target_key)
                                                        {
                                                            s.active_reaction_message = None;
                                                            cx.notify();
                                                            false
                                                        } else {
                                                            s.active_reaction_message =
                                                                Some(target_key);
                                                            cx.notify();
                                                            true
                                                        }
                                                    });
                                                    if opened {
                                                        this.reaction_input
                                                            .update(cx, |input, cx| {
                                                                input.clear(cx)
                                                            });
                                                        window.focus(
                                                            &this.reaction_input.focus_handle(cx),
                                                        );
                                                    }
                                                },
                                            ),
                                        ),
                                )
                            })
                            .when(is_mine && !is_archived, |d| {
                                d.child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-edit-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::TEXT_MUTED)
                                        .child("編集")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let m_id = msg_id_edit.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.start_edit(m_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-delete-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme::BG_SECONDARY)
                                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme::DANGER)
                                        .child("削除")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                                let m_id = msg_id_delete.clone();
                                                this.state.update(cx, |s, cx| {
                                                    s.request_delete(m_id);
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            }),
                    ),
            )
            // MFM 本文
            .child(mfm_view::render_mfm(
                &format!("{pane_prefix}-{msg_id}"),
                &message.content,
                Some(me_user_name),
            ))
            // リアクションピル一覧
            .when(!reaction_pills.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_1p5()
                        .mt_1()
                        .children(reaction_pills),
                )
            })
            // インラインリアクションピッカー
            .when(reaction_picker_open, |d| {
                d.child(
                    div()
                        .mt_1()
                        .p_2()
                        .rounded_md()
                        .bg(theme::BG_SECONDARY)
                        .border_1()
                        .border_color(theme::BORDER_ACCENT)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .children(quick_emoji_buttons),
                        )
                        .child(self.reaction_input.clone()),
                )
            })
            // スレッド返信数ボタン
            .when_some(reply_count.filter(|&c| c > 0), |d, count| {
                d.child(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "btn-open-thread-{}",
                            message.id
                        ))))
                        .mt_1()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::ACCENT)
                        .child(format!("💬 返信 {count} 件を表示"))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let r_id = msg_id_thread.clone();
                                this.state.update(cx, |s, cx| {
                                    s.open_thread(r_id);
                                    cx.notify();
                                });
                            }),
                        ),
                )
            })
    }

    fn render_composer_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let is_synced = state.store.is_synced(&ShapeId::Channels)
            && state.store.is_synced(&ShapeId::ChannelUsers);
        let has_channel = state.current_channel.is_some();
        let is_archived = state
            .current_channel
            .as_deref()
            .and_then(|id| state.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some());
        let can_post = state
            .current_channel
            .as_deref()
            .is_some_and(|id| state.can_post_in_channel(id));

        let (target_label, show_cancel, typing_scope) = match &state.target {
            Target::Channel => {
                let label = state
                    .current_channel
                    .as_deref()
                    .and_then(|id| state.store.channel(id))
                    .map(|ch| format!("{} へ送信", state.store.channel_label(ch, &me_id)))
                    .unwrap_or_else(|| "メッセージ入力".to_string());
                (label, false, None)
            }
            Target::Reply(root_id) => (
                "💬 スレッドに返信中".to_string(),
                true,
                Some(root_id.clone()),
            ),
            Target::Edit(_) => ("✏️ メッセージを編集中".to_string(), true, None),
        };

        let is_editing = matches!(state.target, Target::Edit(_));
        let is_posting_restricted =
            is_synced && has_channel && !is_archived && !can_post && !is_editing;
        let show_composer = !is_archived && (can_post || is_editing);
        let typing_text = state.typing_text(&typing_scope);
        let notice = state.notice.as_ref().map(|(msg, _)| msg.clone());
        let invalid_rows = state.store.invalid_rows;
        let update_available = state.update_available.clone();

        div()
            .w_full()
            .p_3()
            .bg(theme::BG_SECONDARY)
            .border_t_1()
            .border_color(theme::BORDER)
            .flex()
            .flex_col()
            .gap_2()
            .when(is_archived, |d| {
                d.child(
                    div()
                        .w_full()
                        .p_2p5()
                        .rounded_md()
                        .bg(theme::BG_SURFACE)
                        .border_1()
                        .border_color(theme::BORDER)
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child(
                            "このチャンネルはアーカイブされているため、メッセージの投稿や編集はできません",
                        ),
                )
            })
            .when(is_posting_restricted, |d| {
                d.child(
                    div()
                        .w_full()
                        .p_2p5()
                        .rounded_md()
                        .bg(theme::BG_SURFACE)
                        .border_1()
                        .border_color(theme::BORDER)
                        .text_xs()
                        .text_color(theme::TEXT_MUTED)
                        .child("このチャンネルは管理者のみ投稿できます"),
                )
            })
            .when(show_composer, |d| {
                d
                    // 送信先バナー + 入力中インジケーター
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if show_cancel {
                                                theme::ACCENT
                                            } else {
                                                theme::TEXT_MUTED
                                            })
                                            .child(target_label),
                                    )
                                    .when(show_cancel, |d| {
                                        d.child(
                                            div()
                                                .id("btn-cancel-target")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(theme::BG_SURFACE)
                                                .border_1()
                                                .border_color(theme::BORDER)
                                                .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme::TEXT_MUTED)
                                                .child("キャンセル (Esc)")
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                        this.state.update(cx, |s, cx| {
                                                            s.cancel_target();
                                                            cx.notify();
                                                        });
                                                    }),
                                                ),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .when_some(typing_text, |d, t| {
                                        d.child(div().text_xs().text_color(theme::ACCENT).child(t))
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::TEXT_DIM)
                                            .child("Enter: 送信 / Shift+Enter: 改行"),
                                    ),
                            ),
                    )
                    // 入力欄 + 送信ボタン
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_end()
                            .gap_2()
                            .child(div().flex_1().child(self.composer_input.clone()))
                            .child(
                                div()
                                    .id("btn-send-message")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(theme::ACCENT_BUTTON)
                                    .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child(if is_editing { "保存" } else { "送信" })
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.submit_composer(cx);
                                        }),
                                    ),
                            ),
                    )
            })
            // ステータス通知行
            .when(
                notice.is_some() || invalid_rows > 0 || update_available.is_some(),
                |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .text_xs()
                            .child(
                                div()
                                    .text_color(theme::WARNING)
                                    .child(notice.unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_3()
                                    .when(invalid_rows > 0, |d| {
                                        d.child(
                                            div()
                                                .text_color(theme::DANGER)
                                                .child(format!("読めない行: {invalid_rows}")),
                                        )
                                    })
                                    .when_some(update_available, |d, ver| {
                                        d.child(
                                            div()
                                                .text_color(theme::SUCCESS)
                                                .child(format!("新しい版 v{ver} があります")),
                                        )
                                    }),
                            ),
                    )
                },
            )
    }

    fn render_thread_view(&self, root_id: &str, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let Some(channel_id) = state.current_channel.clone() else {
            return div();
        };
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_user_name = state
            .me
            .as_ref()
            .map(|m| m.user_name.clone())
            .unwrap_or_default();

        let is_archived = state
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some());
        let can_post = state.can_post_in_channel(&channel_id);
        let reactions_by_msg = state.channel_cache.reactions_by_message.clone();
        let thread_typing = state.typing_text(&Some(root_id.to_string()));
        let root_message = state.channel_cache.thread_root_message.clone();
        let replies = state.channel_cache.thread_replies.clone();

        let mut cards = Vec::new();
        if let Some(root) = &root_message {
            let root_reactions = reactions_by_msg.get(&root.id).cloned().unwrap_or_default();
            cards.push(self.render_message_card(
                root,
                &me_id,
                &me_user_name,
                is_archived,
                root_reactions,
                None,
                "thread",
                cx,
            ));

            cards.push(
                div()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::TEXT_MUTED)
                    .child(format!("返信 ({} 件)", replies.len())),
            );
            for reply in replies.iter() {
                let r_reactions = reactions_by_msg.get(&reply.id).cloned().unwrap_or_default();
                cards.push(self.render_message_card(
                    reply,
                    &me_id,
                    &me_user_name,
                    is_archived,
                    r_reactions,
                    None,
                    "thread",
                    cx,
                ));
            }
        } else {
            cards.push(
                div()
                    .text_sm()
                    .text_color(theme::TEXT_MUTED)
                    .child("元のメッセージは削除されました"),
            );
        }

        let root_for_focus = root_id.to_string();

        div()
            .flex()
            .flex_col()
            .w(px(380.0))
            .h_full()
            .bg(theme::BG_SECONDARY)
            .border_l_1()
            .border_color(theme::BORDER)
            // スレッドヘッダー
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(theme::BG_SURFACE)
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT_PRIMARY)
                            .child("スレッド"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .when(!is_archived && can_post, |d| {
                                d.child(
                                    div()
                                        .id("btn-thread-reply-focus")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme::ACCENT_BUTTON)
                                        .hover(|s| {
                                            s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer()
                                        })
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme::TEXT_PRIMARY)
                                        .child("返信を書く")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let r_id = root_for_focus.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.focus_thread_reply(r_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("btn-close-thread")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme::BG_SECONDARY)
                                    .border_1()
                                    .border_color(theme::BORDER)
                                    .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme::TEXT_MUTED)
                                    .child("× 閉じる")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.close_thread();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            ),
                    ),
            )
            // スレッド本文一覧
            .child(
                div()
                    .id("thread-scroll-area")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.thread_scroll)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(cards),
            )
            // スレッド内の入力中表示
            .when_some(thread_typing, |d, t| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(theme::BORDER)
                        .text_xs()
                        .text_color(theme::ACCENT)
                        .child(t),
                )
            })
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let login_phase = self.state.read(cx).login_phase.clone();
        if let Some(phase) = login_phase {
            return self.render_login_view(&phase, window, cx);
        }

        // ウィンドウのアクティブ状態とタイムライン・スレッドのスクロール位置を AppState に同期し、
        // ウィンドウがアクティブかつ最下部を見ているときだけ最新メッセージを既読扱いにする。
        let window_active = window.is_window_active();
        let (scroll_to_bottom, thread_scroll_to_bottom) = self.state.update(cx, |s, _| {
            s.ensure_channel_cache();
            (
                std::mem::take(&mut s.pending_scroll_to_bottom),
                std::mem::take(&mut s.pending_thread_scroll_to_bottom),
            )
        });
        if scroll_to_bottom {
            self.timeline_scroll.scroll_to_bottom();
            self.state
                .update(cx, |s, _| s.update_timeline_view_state(window_active, true));
        } else {
            let max_y = self.timeline_scroll.max_offset().height;
            let cur_y = (-self.timeline_scroll.offset().y).max(px(0.0));
            let at_bottom = max_y <= px(4.0) || (max_y - cur_y) <= px(24.0);
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, at_bottom)
            });
        }
        if thread_scroll_to_bottom {
            self.thread_scroll.scroll_to_bottom();
            self.state
                .update(cx, |s, _| s.update_thread_scroll_state(true));
        } else {
            let max_y = self.thread_scroll.max_offset().height;
            let cur_y = (-self.thread_scroll.offset().y).max(px(0.0));
            let at_bottom = max_y <= px(4.0) || (max_y - cur_y) <= px(24.0);
            self.state
                .update(cx, |s, _| s.update_thread_scroll_state(at_bottom));
        }

        let thread_root = self
            .state
            .read(cx)
            .thread
            .as_ref()
            .map(|t| t.root_id.clone());

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(theme::BG_PRIMARY)
            .text_color(theme::TEXT_PRIMARY)
            .track_focus(&self.focus_handle(cx))
            .child(self.render_sidebar_view(cx))
            .child(self.render_timeline_view(cx))
            .when_some(thread_root, |d, root_id| {
                d.child(self.render_thread_view(&root_id, cx))
            })
    }
}

fn render_date_separator(date_label: &str) -> Div {
    div()
        .w_full()
        .py_1()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .child(div().flex_1().h(px(1.0)).bg(theme::BORDER))
        .child(
            div()
                .text_xs()
                .text_color(theme::TEXT_MUTED)
                .child(date_label.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(theme::BORDER))
}

/// UTC の naive 日時をローカルタイムゾーンへ変換する。
pub fn local_zoned(at: DateTime) -> Option<Zoned> {
    at.to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.with_time_zone(TimeZone::system()))
}

pub fn local_time_string(at: DateTime) -> String {
    local_zoned(at)
        .map(|z| z.strftime("%H:%M").to_string())
        .unwrap_or_default()
}

fn is_activate_key(key: &str) -> bool {
    matches!(key, "enter" | "space" | " ")
}
