//! タイムライン (frontend の `ChannelScreen` / `ChannelThreadItems` に対応)。

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseUpEvent, ParentElement, StatefulInteractiveElement,
    Styled, div, prelude::*, px,
};
use zoo2_client_core::electric::ShapeId;

use crate::theme;
use crate::view::{WorkspaceView, local_zoned};

impl WorkspaceView {
    pub(crate) fn render_timeline_view(
        &self,
        pending_scroll_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>) {
        let (
            channel_id,
            me_id,
            me_user_name,
            ch_label,
            ch_desc,
            is_archived,
            is_member,
            can_join,
            can_leave,
            failed_draft_count,
            confirm_delete_id,
            synced,
            timeline_at_bottom,
            all_root_messages,
            reply_counts,
            reactions_by_msg,
            attachments_by_msg,
            is_searching,
            start_idx,
            end_idx,
            theme,
        ) = {
            let state = self.state.read(cx);
            let theme = state.theme();
            let Some(channel_id) = state.current_channel.clone() else {
                return (
                    div()
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
                                .text_color(theme.text_primary)
                                .child("チャンネルを選択してください"),
                        )
                        .child(div().text_sm().text_color(theme.text_muted).child(
                            "左のサイドバーからチャンネルまたはダイレクトメッセージを開けます。",
                        )),
                    None,
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
            let reply_counts = state.channel_cache.reply_counts.clone();
            let reactions_by_msg = state.channel_cache.reactions_by_message.clone();
            let attachments_by_msg = state.channel_cache.attachments_by_message.clone();
            let is_searching = state.message_search.is_some();

            (
                channel_id,
                me_id,
                me_user_name,
                ch_label,
                ch_desc,
                is_archived,
                is_member,
                can_join,
                can_leave,
                failed_draft_count,
                confirm_delete_id,
                synced,
                timeline_at_bottom,
                all_root_messages,
                reply_counts,
                reactions_by_msg,
                attachments_by_msg,
                is_searching,
                start_idx,
                end_idx,
                theme,
            )
        };

        let hidden_before = start_idx;
        let hidden_after = all_root_messages.len().saturating_sub(end_idx);
        let visible_roots = &all_root_messages[start_idx..end_idx];

        let mut timeline_items: Vec<Div> = Vec::new();
        if hidden_before > 0 {
            timeline_items.push(
                div().w_full().flex().justify_center().py_1().child(
                    div()
                        .id("btn-load-older-messages")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .text_color(theme.accent)
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
        let mut target_scroll_item_index = None;
        let mut last_date = None;
        for msg in visible_roots {
            let date = local_zoned(msg.created_at).map(|z| z.date());
            if date.is_some() && date != last_date {
                if let Some(d) = date {
                    timeline_items.push(render_date_separator(&d.to_string(), &theme));
                }
                last_date = date;
            }
            if pending_scroll_id == Some(msg.id.as_str()) {
                target_scroll_item_index = Some(timeline_items.len());
            }
            let reply_count = reply_counts.get(&msg.id).copied().unwrap_or(0);
            let reactions = reactions_by_msg.get(&msg.id).cloned().unwrap_or_default();
            let attachments = attachments_by_msg
                .get(&msg.id)
                .map(|v| v.as_slice())
                .unwrap_or_default();
            timeline_items.push(self.render_message_card(
                msg,
                &me_id,
                &me_user_name,
                is_archived,
                reactions,
                attachments,
                Some(reply_count),
                "timeline",
                cx,
            ));
        }

        let ch_id_for_join = channel_id.clone();
        let theme = self.state.read(cx).theme();

        let timeline_view = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            // チャンネルヘッダー
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(theme.bg_surface)
                    .border_b_1()
                    .border_color(theme.border)
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
                                    .text_color(theme.text_primary)
                                    .child(ch_label),
                            )
                            .when_some(ch_desc, |d, desc| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_muted)
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
                                        .bg(theme.bg_secondary)
                                        .text_xs()
                                        .text_color(theme.warning)
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
                                        .bg(theme.bg_secondary)
                                        .border_1()
                                        .border_color(theme.warning)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.warning)
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
                                            theme.bg_secondary
                                        } else {
                                            theme.accent_button
                                        })
                                        .border_1()
                                        .border_color(theme.border)
                                        .hover(move |s| {
                                            s.bg(if is_member {
                                                theme.bg_hover
                                            } else {
                                                theme.accent_button_hover
                                            })
                                            .cursor_pointer()
                                        })
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if is_member {
                                            theme.text_primary
                                        } else {
                                            theme.text_on_accent
                                        })
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
                            })
                            .child(
                                div()
                                    .id("btn-open-search")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("🔍 検索")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                                            this.toggle_message_search(window, cx);
                                        }),
                                    ),
                            ),
                    ),
            )
            // 検索バー
            .when(is_searching, |d| {
                d.child(self.render_message_search_bar(cx))
            })
            // 削除確認バー
            .when(confirm_delete_id.is_some(), |d| {
                d.child(
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
                                .text_color(theme.text_muted)
                                .child("メッセージを読み込み中…"),
                        )
                    })
                    .when(synced && timeline_items.is_empty(), |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
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
                        .bg(theme.bg_surface)
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("btn-jump-to-latest")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(theme.accent_button)
                                .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_on_accent)
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
            .child(self.render_composer_view(cx));

        (timeline_view, target_scroll_item_index)
    }
}

fn render_date_separator(date_label: &str, theme: &theme::Theme) -> Div {
    div()
        .w_full()
        .py_1()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(date_label.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
}
