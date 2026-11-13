//! チャンネルヘッダー (frontend の `chat/ChannelHeader.tsx` に対応)。

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseUpEvent, ParentElement, Styled, div, prelude::*,
};

use crate::theme::Theme;
use crate::view::WorkspaceView;

impl WorkspaceView {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn render_channel_header(
        &self,
        ch_label: String,
        ch_desc: Option<String>,
        is_archived: bool,
        failed_draft_count: usize,
        can_join: bool,
        can_leave: bool,
        is_member: bool,
        ch_id_for_join: String,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
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
            )
    }
}
