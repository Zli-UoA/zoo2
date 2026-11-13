//! スレッドペイン。

use gpui::{
    Context, Div, Focusable, FontWeight, MouseButton, MouseUpEvent, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn render_thread_view(
        &self,
        root_id: &str,
        pending_scroll_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>) {
        let state = self.state.read(cx);
        let theme = state.theme();
        let Some(channel_id) = state.current_channel.clone() else {
            return (div(), None);
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
        let attachments_by_msg = state.channel_cache.attachments_by_message.clone();
        let thread_typing = state.typing_text(&Some(root_id.to_string()));
        let root_message = state.channel_cache.thread_root_message.clone();
        let replies = state.channel_cache.thread_replies.clone();

        let mut target_scroll_index = None;
        let mut cards = Vec::new();
        if let Some(root) = &root_message {
            let root_reactions = reactions_by_msg.get(&root.id).cloned().unwrap_or_default();
            let root_attachments = attachments_by_msg
                .get(&root.id)
                .map(|v| v.as_slice())
                .unwrap_or_default();
            cards.push(self.render_message_card(
                root,
                &me_id,
                &me_user_name,
                is_archived,
                root_reactions,
                root_attachments,
                None,
                "thread",
                cx,
            ));

            cards.push(
                div()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_muted)
                    .child(format!("返信 ({} 件)", replies.len())),
            );
            for reply in replies.iter() {
                if pending_scroll_id == Some(reply.id.as_str()) {
                    target_scroll_index = Some(cards.len());
                }
                let r_reactions = reactions_by_msg.get(&reply.id).cloned().unwrap_or_default();
                let r_attachments = attachments_by_msg
                    .get(&reply.id)
                    .map(|v| v.as_slice())
                    .unwrap_or_default();
                cards.push(self.render_message_card(
                    reply,
                    &me_id,
                    &me_user_name,
                    is_archived,
                    r_reactions,
                    r_attachments,
                    None,
                    "thread",
                    cx,
                ));
            }
        } else {
            cards.push(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child("元のメッセージは削除されました"),
            );
        }

        let root_for_focus = root_id.to_string();

        let thread_view = div()
            .flex()
            .flex_col()
            .w(px(380.0))
            .h_full()
            .bg(theme.bg_secondary)
            .border_l_1()
            .border_color(theme.border)
            // スレッドヘッダー
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
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
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
                                        .bg(theme.accent_button)
                                        .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_on_accent)
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
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
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
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.accent)
                        .child(t),
                )
            });

        (thread_view, target_scroll_index)
    }
}
