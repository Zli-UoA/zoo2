//! メッセージ行 (frontend の `components/chat/messages/**` に対応)。

use gpui::{
    Context, Div, ElementId, Focusable, FontWeight, MouseButton, MouseUpEvent, ParentElement,
    SharedString, Styled, div, prelude::*,
};
use zoo2_client_core::{
    electric::rows::{Attachment, Message, parse_message_cards},
    store::ReactionGroup,
};

use crate::view::{WorkspaceView, local_time_string};
use crate::view::{render_attachment_item, render_ogp_card};
use crate::{
    app::{QUICK_EMOJIS, reaction_label},
    mfm_view,
};

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_message_card(
        &self,
        message: &Message,
        me_id: &str,
        me_user_name: &str,
        is_archived: bool,
        reactions: Vec<ReactionGroup>,
        attachments: &[Attachment],
        reply_count: Option<usize>,
        pane_prefix: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let msg_id = message.id.clone();
        let body = mfm_view::render_mfm_accounts_with_theme(
            &format!("{pane_prefix}-{msg_id}"),
            &message.content,
            Some(me_user_name),
            &theme,
            &state.store.account_profiles,
        );
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
                theme.bg_reaction_mine
            } else {
                theme.bg_secondary
            };
            let pill_border = if group.mine {
                theme.border_accent
            } else {
                theme.border
            };
            let pill_text = if group.mine {
                theme.accent
            } else {
                theme.text_muted
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
                    d.hover(|s| s.bg(theme.bg_hover).cursor_pointer())
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
                        .bg(theme.bg_primary)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
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

        let is_search_matched = state
            .message_search
            .as_ref()
            .is_some_and(|ms| ms.matches.contains(&msg_id));
        let is_current_search_match = state
            .message_search
            .as_ref()
            .and_then(|ms| ms.matches.get(ms.current_match_index))
            == Some(&msg_id);

        let card_border_color = if is_current_search_match {
            theme.accent
        } else if is_search_matched {
            theme.border_accent
        } else {
            theme.border
        };
        let card_bg = if is_current_search_match {
            theme.bg_mention_mine
        } else if is_search_matched {
            theme.bg_selected
        } else {
            theme.bg_surface
        };

        let attachments_with_urls: Vec<(Attachment, Option<String>, bool, bool, bool)> =
            attachments
                .iter()
                .map(|att| {
                    let url = state.attachment_download_url(&att.id);
                    let is_fetching = state.fetching_attachment_urls.contains(&att.id)
                        || state.pending_open_attachments.contains(&att.id);
                    let needs_refresh = state.attachment_url_needs_refresh(&att.id);
                    let is_failed = state.failed_attachment_urls.contains_key(&att.id);
                    (att.clone(), url, is_fetching, needs_refresh, is_failed)
                })
                .collect();
        let attachment_views: Vec<Div> = attachments_with_urls
            .into_iter()
            .map(|(att, url, is_fetching, needs_refresh, is_failed)| {
                render_attachment_item(
                    &att,
                    url,
                    is_fetching,
                    needs_refresh,
                    is_failed,
                    &theme,
                    &self.attachment_focus_handles,
                    cx,
                )
            })
            .collect();

        let ogp_cards = message
            .card
            .as_deref()
            .map(parse_message_cards)
            .unwrap_or_default();
        let ogp_views: Vec<Div> = ogp_cards
            .iter()
            .enumerate()
            .map(|(idx, card)| {
                let card_key = format!("{}:{idx}", message.id);
                let is_expanded = self.expanded_cards.contains(&card_key);
                render_ogp_card(
                    card,
                    &card_key,
                    is_expanded,
                    &self.card_focus_handles,
                    &theme,
                    cx,
                )
            })
            .collect();

        div()
            .w_full()
            .p_3()
            .rounded_md()
            .bg(card_bg)
            .border_1()
            .border_color(card_border_color)
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
                                    .text_color(theme.text_primary)
                                    .child(author_name),
                            )
                            .when(is_bot, |d| {
                                d.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("BOT"),
                                )
                            })
                            .child(div().text_xs().text_color(theme.text_muted).child(time_str))
                            .when(is_edited, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_dim)
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
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
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
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
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
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
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
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.danger)
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
            .child(body)
            // 添付ファイル (画像・ファイル)
            .when(!attachment_views.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .mt_1()
                        .children(attachment_views),
                )
            })
            // OGP リンクカード
            .when(!ogp_views.is_empty(), |d| {
                d.child(div().flex().flex_col().gap_1p5().mt_1().children(ogp_views))
            })
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
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.border_accent)
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
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent)
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
}
