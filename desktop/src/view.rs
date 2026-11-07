//! GPUI による画面描画 (`WorkspaceView`、`LoginView`、`SidebarView`、`TimelineView`、`ComposerView`、`ThreadView`)。

use std::collections::HashMap;

use gpui::{
    ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, MouseUpEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
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
    focus_handle: FocusHandle,
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
                        let Some(msg_id) = state.active_reaction_message.clone() else {
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
            let pending = state.update(cx, |s, _| s.pending_composer_text.take());
            if let Some(text) = pending {
                this.composer_input
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
            focus_handle: cx.focus_handle(),
        }
    }

    fn submit_composer(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_input.read(cx).text().to_string();
        let submitted = self.state.update(cx, |state, cx| {
            let ok = state.submit_message(text);
            cx.notify();
            ok
        });
        if submitted {
            self.composer_input.update(cx, |input, cx| input.clear(cx));
        }
    }

    fn render_login_view(&self, phase: &LoginPhase, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let api_url = state.api_base_url.clone();
        let auth_summary = state.auth_summary.clone();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));

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
                    col = col
                        .child(
                            div()
                                .id("btn-start-device-login")
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme::ACCENT_BUTTON)
                                .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::TEXT_PRIMARY)
                                .child("ブラウザでログイン")
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
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme::BG_SURFACE)
                                .border_1()
                                .border_color(theme::BORDER)
                                .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .text_color(theme::TEXT_MUTED)
                                .child("保存済みトークンで再試行")
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
                    col = col.child(
                        div()
                            .id("btn-retry-dev-auth")
                            .w_full()
                            .py_2()
                            .px_4()
                            .rounded_md()
                            .bg(theme::ACCENT_BUTTON)
                            .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme::TEXT_PRIMARY)
                            .child("再接続する")
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
                let code_for_copy = user_code.clone();
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
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme::ACCENT_BUTTON)
                                    .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child("ブラウザを開く")
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
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme::BG_SURFACE)
                                    .border_1()
                                    .border_color(theme::BORDER)
                                    .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child("コードをコピー")
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
                            .w_full()
                            .py_1p5()
                            .px_3()
                            .rounded_md()
                            .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                            .text_center()
                            .text_xs()
                            .text_color(theme::TEXT_MUTED)
                            .child("キャンセル")
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

        div()
            .flex()
            .size_full()
            .bg(theme::BG_PRIMARY)
            .text_color(theme::TEXT_PRIMARY)
            .justify_center()
            .items_center()
            .p_6()
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
                            } else {
                                "チャンネル (同期中…)"
                            }),
                    )
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
        let is_dm = channel.is_some_and(|c| c.is_dm);
        let is_archived = channel.is_some_and(|c| c.archive_at.is_some());
        let is_member = state.store.is_member(&channel_id, &me_id);
        let failed_draft_count = state.failed_drafts.len();
        let confirm_delete_id = state.confirm_delete_message.clone();
        let synced = state
            .store
            .is_synced(&ShapeId::Messages(channel_id.clone()));

        let root_messages: Vec<Message> = state
            .store
            .root_messages(&channel_id)
            .into_iter()
            .cloned()
            .collect();
        let reply_counts = state.store.reply_counts(&channel_id);
        let reactions_by_msg = state.store.reaction_groups_by_message(&channel_id, &me_id);

        let mut timeline_items: Vec<Div> = Vec::new();
        let mut last_date = None;
        for msg in &root_messages {
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
                            .when(!is_dm, |d| {
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
            // 下部入力欄
            .child(self.render_composer_view(cx))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_message_card(
        &self,
        message: &Message,
        me_id: &str,
        me_user_name: &str,
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
        let reaction_picker_open =
            state.active_reaction_message.as_deref() == Some(msg_id.as_str());

        let msg_id_reply = msg_id.clone();
        let msg_id_react = msg_id.clone();
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
            reaction_pills.push(
                div()
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
                    .hover(|s| s.bg(theme::BG_HOVER).cursor_pointer())
                    .text_xs()
                    .text_color(pill_text)
                    .child(format!("{} {}", reaction_label(&group.emoji), group.count))
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
                    ),
            );
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
                            .child(
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
                                        cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                                            let r_id = root_for_reply_btn.clone();
                                            this.state.update(cx, |s, cx| {
                                                s.open_thread(r_id);
                                                cx.notify();
                                            });
                                            window.focus(&this.composer_input.focus_handle(cx));
                                        }),
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
                                        cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                                            let m_id = msg_id_react.clone();
                                            let opened = this.state.update(cx, |s, cx| {
                                                if s.active_reaction_message.as_deref()
                                                    == Some(m_id.as_str())
                                                {
                                                    s.active_reaction_message = None;
                                                    cx.notify();
                                                    false
                                                } else {
                                                    s.active_reaction_message = Some(m_id);
                                                    cx.notify();
                                                    true
                                                }
                                            });
                                            if opened {
                                                this.reaction_input
                                                    .update(cx, |input, cx| input.clear(cx));
                                                window.focus(&this.reaction_input.focus_handle(cx));
                                            }
                                        }),
                                    ),
                            )
                            .when(is_mine, |d| {
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
            .child(mfm_view::render_mfm(&message.content, Some(me_user_name)))
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

        let reactions_by_msg: HashMap<String, Vec<ReactionGroup>> =
            state.store.reaction_groups_by_message(&channel_id, &me_id);
        let thread_typing = state.typing_text(&Some(root_id.to_string()));
        let root_message = state.store.message(&channel_id, root_id).cloned();
        let replies: Vec<Message> = state
            .store
            .replies(&channel_id, root_id)
            .into_iter()
            .cloned()
            .collect();

        let mut cards = Vec::new();
        if let Some(root) = &root_message {
            let root_reactions = reactions_by_msg.get(&root.id).cloned().unwrap_or_default();
            cards.push(self.render_message_card(
                root,
                &me_id,
                &me_user_name,
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
            for reply in &replies {
                let r_reactions = reactions_by_msg.get(&reply.id).cloned().unwrap_or_default();
                cards.push(self.render_message_card(
                    reply,
                    &me_id,
                    &me_user_name,
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
                            .child(
                                div()
                                    .id("btn-thread-reply-focus")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme::ACCENT_BUTTON)
                                    .hover(|s| s.bg(theme::ACCENT_BUTTON_HOVER).cursor_pointer())
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::TEXT_PRIMARY)
                                    .child("返信を書く")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                                            let r_id = root_for_focus.clone();
                                            this.state.update(cx, |s, cx| {
                                                s.target = Target::Reply(r_id);
                                                cx.notify();
                                            });
                                            window.focus(&this.composer_input.focus_handle(cx));
                                        }),
                                    ),
                            )
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let login_phase = self.state.read(cx).login_phase.clone();
        if let Some(phase) = login_phase {
            return self.render_login_view(&phase, cx);
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
