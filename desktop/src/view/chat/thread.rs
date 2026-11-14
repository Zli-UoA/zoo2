//! スレッドペイン (frontend の `ThreadPane.tsx` / `ThreadPaneStatus.tsx`)。
//!
//! 写していないもの: ペイン内の入力欄 (入力欄はタイムラインの下にひとつだけあり、スレッドを開くと返信先になる。
//! ヘッダーの「返信を書く」はその入力欄へ移るための、デスクトップだけのボタン)、
//! 親投稿・返信の取得失敗と再試行 (`isError` / `isOffline` に当たる状態がない)、狭い幅での全画面表示。

use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyElement, Context, Div, Focusable, ParentElement, Styled, Window, div, prelude::*, px,
};

use crate::icons::Icon;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::pane_header::PaneHeader;
use crate::ui::scroll_area::ScrollArea;
use crate::ui::{TextSpec, alpha, h_flex, palette, v_flex};
use crate::view::WorkspaceView;
use crate::view::chat::message::RowList;

/// ペインの幅 (`sm:w-80`)。
pub const THREAD_WIDTH: f32 = 320.0;
/// 幅が広いときのペインの幅 (`xl:w-96`)。
pub const THREAD_WIDTH_WIDE: f32 = 384.0;
/// `xl` のブレークポイント。
pub const XL_BREAKPOINT: f32 = 1280.0;

/// ビューポートの幅からペインの幅を決める (`sm:w-80 xl:w-96`)。
pub fn pane_width(viewport_width: f32) -> f32 {
    if viewport_width >= XL_BREAKPOINT {
        THREAD_WIDTH_WIDE
    } else {
        THREAD_WIDTH
    }
}

impl WorkspaceView {
    pub(in crate::view) fn render_thread_view(
        &self,
        root_id: &str,
        pending_scroll_id: Option<&str>,
        channels: Rc<HashMap<String, String>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>) {
        let t = palette(cx).t;
        let i18n = self.i18n;
        let xs = TextSpec::XS;
        let sm = TextSpec::SM;
        let width = pane_width(f32::from(window.viewport_size().width));
        let state = self.state.read(cx);
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

        // 親投稿。`max-h-60 shrink-0 overflow-y-auto border-b bg-muted/30 px-2 py-2`
        let root_block = root_message.as_ref().map(|root| {
            let (rows, _) = self.render_message_rows(
                RowList {
                    messages: vec![root],
                    first_unread_id: None,
                    me_id: &me_id,
                    me_user_name: &me_user_name,
                    is_archived,
                    reactions: &reactions_by_msg,
                    attachments: &attachments_by_msg,
                    reply_counts: None,
                    pane_prefix: "thread",
                    scroll_target: None,
                    solo: true,
                },
                channels.clone(),
                cx,
            );
            div()
                .id("thread-root-block")
                .flex_none()
                .max_h(px(240.0))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(t.border)
                .bg(alpha(t.muted, 0.3))
                .py(px(8.0))
                .children(rows)
        });

        // 返信の一覧。`min-h-0 flex-1 overflow-y-auto px-2 py-2`
        let mut target_scroll_index = None;
        let mut items: Vec<AnyElement> = vec![div().h(px(8.0)).flex_none().into_any_element()];
        if root_message.is_none() {
            items.push(
                div()
                    .p(px(12.0))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(t.muted_foreground)
                    .child(i18n.t("threads.rootDeleted"))
                    .into_any_element(),
            );
        } else if replies.is_empty() {
            items.push(
                div()
                    .p(px(12.0))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(t.muted_foreground)
                    .child(i18n.t("threads.empty"))
                    .into_any_element(),
            );
        } else {
            let (rows, target) = self.render_message_rows(
                RowList {
                    messages: replies.iter().collect(),
                    first_unread_id: None,
                    me_id: &me_id,
                    me_user_name: &me_user_name,
                    is_archived,
                    reactions: &reactions_by_msg,
                    attachments: &attachments_by_msg,
                    reply_counts: None,
                    pane_prefix: "thread",
                    scroll_target: pending_scroll_id,
                    solo: false,
                },
                channels.clone(),
                cx,
            );
            target_scroll_index = target.map(|i| items.len() + i);
            items.extend(rows);
        }
        items.push(div().h(px(8.0)).flex_none().into_any_element());

        let root_for_focus = root_id.to_string();
        let reply_count_label = i18n.t_count("threads.replyCount", replies.len() as i64, &[]);

        let header = PaneHeader::new()
            .gap(px(8.0))
            .child(
                // `min-w-0 flex-1`: `h2 text-sm font-semibold` と `p text-xs tabular-nums text-muted-foreground`
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(sm.size())
                            .line_height(sm.line())
                            .font(crate::fonts::sans(600.0))
                            .child(i18n.t("threads.title")),
                    )
                    .child(
                        div()
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .text_color(t.muted_foreground)
                            .child(reply_count_label),
                    ),
            )
            .when(is_archived, |d| {
                d.child(
                    div()
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.muted_foreground)
                        .child(i18n.t("moderation.archived")),
                )
            })
            .when(!is_archived && can_post, |d| {
                d.child(
                    Button::new("btn-thread-reply-focus")
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .label("返信を書く")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let r_id = root_for_focus.clone();
                            this.state.update(cx, |s, cx| {
                                s.focus_thread_reply(r_id);
                                cx.notify();
                            });
                            window.focus(&this.composer_input.focus_handle(cx));
                        })),
                )
            })
            .child(
                Button::new("btn-close-thread")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon_start(Icon::X)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.close_thread();
                            cx.notify();
                        });
                    })),
            );

        // `flex min-h-0 min-w-0 flex-col border-l bg-background sm:w-80 sm:shrink-0 xl:w-96`
        let thread_view = v_flex()
            .flex_none()
            .w(px(width))
            .h_full()
            .min_h_0()
            .min_w_0()
            .border_l_1()
            .border_color(t.border)
            .bg(t.background)
            .child(header)
            .children(root_block)
            .child(
                div().relative().flex_1().min_h_0().child(
                    ScrollArea::new("thread-scroll-area")
                        .scroll_handle(&self.thread_scroll)
                        .size_full()
                        .children(items),
                ),
            )
            // `TypingIndicator`: `min-h-6 px-4 text-xs text-muted-foreground`
            .child(
                h_flex()
                    .flex_none()
                    .min_h(px(24.0))
                    .px(px(16.0))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(t.muted_foreground)
                    .children(thread_typing),
            );

        (thread_view, target_scroll_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_is_wider_from_the_xl_breakpoint() {
        assert_eq!(pane_width(1279.0), 320.0);
        assert_eq!(pane_width(1280.0), 384.0);
        assert_eq!(pane_width(1440.0), 384.0);
        assert_eq!(pane_width(800.0), 320.0);
    }
}
