//! コンポーザー (frontend の `chat/messages/Composer.tsx` / `ComposerField.tsx` に対応)。

use gpui::{
    Context, Div, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    PathPromptOptions, StatefulInteractiveElement, Styled, div, prelude::*, px,
};
use zoo2_client_core::electric::ShapeId;

use crate::app::Target;
use crate::view::format_file_size;
use crate::view::{WorkspaceView, is_activate_key};

impl WorkspaceView {
    pub(in crate::view) fn submit_composer(&mut self, cx: &mut Context<Self>) {
        self.composer_input
            .update(cx, |input, cx| input.request_submit(cx));
    }

    pub(in crate::view) fn prompt_and_add_attachments(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(channel_id) = state.current_channel.as_deref() else {
            return;
        };
        if !state.can_post_in_channel(channel_id) {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    this.state.update(cx, |s, cx| {
                        for p in paths {
                            s.add_attachment_from_path(p);
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    pub(in crate::view) fn render_composer_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
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
        // 暗号化チャンネルはタイムラインに「表示できない」と出すので、入力欄も制限の案内も出さない。
        let is_encrypted = state
            .current_channel
            .as_deref()
            .is_some_and(|id| state.is_encrypted_channel(id));
        let is_posting_restricted =
            is_synced && has_channel && !is_archived && !is_encrypted && !can_post && !is_editing;
        let show_composer = !is_archived && !is_encrypted && (can_post || is_editing);
        let typing_text = state.typing_text(&typing_scope);
        let notice = state.notice.as_ref().map(|(msg, _)| msg.clone());
        let invalid_rows = state.store.invalid_rows;
        let update_available = state.update_available.clone();

        let pending_attachments_bar = self.render_pending_attachments_bar(cx);
        let mention_autocomplete_view = self.render_mention_autocomplete(cx);

        div()
            .w_full()
            .p_3()
            .bg(theme.bg_secondary)
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_2()
            .when_some(mention_autocomplete_view, |d, v| d.child(v))
            .when_some(pending_attachments_bar, |d, v| d.child(v))
            .when(is_archived, |d| {
                d.child(
                    div()
                        .w_full()
                        .p_2p5()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.text_muted)
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
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.text_muted)
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
                                                theme.accent
                                            } else {
                                                theme.text_muted
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
                                                .bg(theme.bg_surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme.text_muted)
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
                                        d.child(div().text_xs().text_color(theme.accent).child(t))
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_dim)
                                            .child("Enter: 送信 / Shift+Enter: 改行"),
                                    ),
                            ),
                    )
                    // 入力欄 + 送信ボタン & アクションボタン
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_end()
                            .gap_2()
                            .when(!is_editing, |d| {
                                d.child(
                                    div()
                                        .id("btn-attach-file")
                                        .px_2p5()
                                        .py_2()
                                        .rounded_md()
                                        .bg(theme.bg_surface)
                                        .border_1()
                                        .border_color(theme.border)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_sm()
                                        .child("📎")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.prompt_and_add_attachments(cx);
                                            }),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("btn-open-emoji-picker")
                                    .px_2p5()
                                    .py_2()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_sm()
                                    .child("😀")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                if s.emoji_picker_open {
                                                    s.close_emoji_picker();
                                                } else {
                                                    s.open_emoji_picker();
                                                }
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(div().flex_1().child(self.composer_input.clone()))
                            .child(
                                div()
                                    .id("btn-send-message")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(theme.accent_button)
                                    .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_on_accent)
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
                                    .text_color(theme.warning)
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
                                                .text_color(theme.danger)
                                                .child(format!("読めない行: {invalid_rows}")),
                                        )
                                    })
                                    .when_some(update_available, |d, ver| {
                                        d.child(
                                            div()
                                                .text_color(theme.success)
                                                .child(format!("新しい版 v{ver} があります")),
                                        )
                                    }),
                            ),
                    )
                },
            )
    }

    fn render_pending_attachments_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.state.read(cx);
        if matches!(state.target, Target::Edit(_)) {
            return None;
        }
        let attachments = state.current_channel_attachments();
        if attachments.is_empty() {
            return None;
        }
        let theme = state.theme();
        let pills = attachments.into_iter().map(|att| {
            let local_id = att.local_id.clone();
            let name = att.file_name.clone();
            let size_str = format_file_size(att.file_size);
            let status_text = if att.is_uploading {
                " (アップロード中…)"
            } else if att.error.is_some() {
                " (エラー)"
            } else {
                ""
            };
            let text_color = if att.error.is_some() {
                theme.danger
            } else if att.is_uploading {
                theme.text_muted
            } else {
                theme.success
            };

            let del_focus = self
                .attachment_focus_handles
                .borrow_mut()
                .entry(format!("pending_del:{}", att.local_id))
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            let lid_kd = local_id.clone();

            div()
                .px_2p5()
                .py_1()
                .rounded_md()
                .bg(theme.bg_surface)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(theme.accent).child("📎"))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_primary)
                        .child(format!("{name} ({size_str})")),
                )
                .child(div().text_xs().text_color(text_color).child(status_text))
                .child(
                    div()
                        .track_focus(&del_focus)
                        .focus(|s| s.border_1().border_color(theme.danger).bg(theme.bg_hover))
                        .px_1()
                        .rounded_sm()
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .text_color(theme.danger)
                        .child("✕")
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let lid = local_id.clone();
                                this.state.update(cx, |s, cx| {
                                    s.remove_pending_attachment(&lid);
                                    cx.notify();
                                });
                            }),
                        )
                        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                            if is_activate_key(&ev.keystroke.key) {
                                cx.stop_propagation();
                                let lid = lid_kd.clone();
                                this.state.update(cx, |s, cx| {
                                    s.remove_pending_attachment(&lid);
                                    cx.notify();
                                });
                            }
                        })),
                )
        });

        Some(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .children(pills),
        )
    }

    fn render_mention_autocomplete(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.state.read(cx);
        let theme = state.theme();
        let autocomplete = state.mention_autocomplete.as_ref()?;
        if autocomplete.candidates.is_empty() {
            return None;
        }

        let items = autocomplete
            .candidates
            .iter()
            .enumerate()
            .map(|(i, candidate)| {
                let is_selected = i == autocomplete.selected_index;
                let display = candidate.display_name.clone();
                let idx = i;
                div()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
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
                            .text_xs()
                            .font_weight(if is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if candidate.is_special {
                                theme.accent
                            } else {
                                theme.text_primary
                            })
                            .child(display),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                            this.state.update(cx, |s, cx| {
                                if let Some(ac) = &mut s.mention_autocomplete {
                                    ac.selected_index = idx;
                                }
                                s.mention_confirm();
                                cx.notify();
                            });
                        }),
                    )
            });

        Some(
            div()
                .w_full()
                .p_1p5()
                .rounded_md()
                .bg(theme.bg_surface)
                .border_1()
                .border_color(theme.border_accent)
                .child(
                    div()
                        .id("mention-autocomplete-list")
                        .w_full()
                        .max_h(px(180.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(items),
                ),
        )
    }
}
