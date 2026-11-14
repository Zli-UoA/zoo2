//! タイムライン (frontend の `ChannelScreen` / `ChannelThreadItems` に対応)。

use std::{collections::HashMap, rc::Rc};

use gpui::{AnyElement, Context, Div, ParentElement, Styled, div, prelude::*, px};
use zoo2_client_core::electric::ShapeId;

use crate::i18n::I18n;
use crate::icons::Icon;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::scroll_area::ScrollArea;
use crate::ui::skeleton::Skeleton;
use crate::ui::{Palette, TextSpec, h_flex, icon_colored, palette, shadow_md, v_flex};
use crate::view::WorkspaceView;
use crate::view::chat::channel_header::ChannelHeaderInfo;
use crate::view::chat::message::RowList;
use crate::view::chat::message_format::first_unread_id_in_window;
use crate::view::shell::sidebar::{channel_title, is_group_dm};

impl WorkspaceView {
    pub(in crate::view) fn render_timeline_view(
        &self,
        pending_scroll_id: Option<&str>,
        channels: Rc<HashMap<String, String>>,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>, bool) {
        if self.state.read(cx).current_channel.is_none() {
            return (self.render_welcome_panel(cx), None, true);
        }
        let (
            channel_id,
            me_id,
            me_user_name,
            ch_label,
            ch_desc,
            is_archived,
            header_kind,
            dm_member_count,
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
            read_boundary,
            start_idx,
            end_idx,
        ) = {
            let state = self.state.read(cx);
            let Some(channel_id) = state.current_channel.clone() else {
                // 選択なしは上で `render_welcome_panel` を返している。ここへは来ない。
                return (div(), None, true);
            };

            let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            let me_user_name = state
                .me
                .as_ref()
                .map(|m| m.user_name.clone())
                .unwrap_or_default();
            let channel = state.store.channel(&channel_id);
            let ch_label = channel
                .map(|c| channel_title(state, c, &me_id, &self.i18n))
                .unwrap_or_else(|| format!("#{channel_id}"));
            let ch_desc = channel
                .and_then(|c| c.description.clone())
                .filter(|d| !d.is_empty());
            let is_archived = channel.is_some_and(|c| c.archive_at.is_some());
            let header_kind = channel.map(|c| {
                (
                    c.is_dm,
                    c.is_dm && is_group_dm(&c.channel_name),
                    c.is_private,
                    c.is_nsfw,
                )
            });
            let dm_member_count = if channel.is_some_and(|c| c.is_dm) {
                state.store.channel_members(&channel_id).len()
            } else {
                0
            };
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
            let read_boundary = state.read_boundary.get(&channel_id).copied();

            (
                channel_id,
                me_id,
                me_user_name,
                ch_label,
                ch_desc,
                is_archived,
                header_kind,
                dm_member_count,
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
                read_boundary,
                start_idx,
                end_idx,
            )
        };

        // デスクトップ版は E2EE の対象外。暗号化チャンネルでは、ヘッダー（退出などの操作）は残し、
        // メッセージの一覧の代わりに「表示できない」と出す。
        let is_encrypted = self.state.read(cx).is_encrypted_channel(&channel_id);
        let (hidden_before, hidden_after, visible_roots) = if is_encrypted {
            (0, 0, &all_root_messages[..0])
        } else {
            (
                start_idx,
                all_root_messages.len().saturating_sub(end_idx),
                &all_root_messages[start_idx..end_idx],
            )
        };

        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let xs = TextSpec::XS;
        let sm = TextSpec::SM;

        // `px-2 py-3` の上の余白。
        let mut timeline_items: Vec<AnyElement> =
            vec![div().h(px(12.0)).flex_none().into_any_element()];
        if hidden_before > 0 {
            timeline_items.push(
                h_flex()
                    .justify_center()
                    .py_1()
                    .child(
                        Button::new("btn-load-older-messages")
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .label(format!("過去のメッセージを表示 (残り {hidden_before} 件)"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.update(cx, |s, cx| {
                                    s.load_older_messages();
                                    cx.notify();
                                });
                            })),
                    )
                    .into_any_element(),
            );
        }
        let first_unread = read_boundary
            .and_then(|boundary| {
                first_unread_id_in_window(&all_root_messages, start_idx, &me_id, boundary)
            })
            .map(str::to_string);
        let (rows, target_in_rows) = self.render_message_rows(
            RowList {
                messages: visible_roots.iter().collect(),
                first_unread_id: first_unread.as_deref(),
                me_id: &me_id,
                me_user_name: &me_user_name,
                is_archived,
                reactions: &reactions_by_msg,
                attachments: &attachments_by_msg,
                reply_counts: Some(&reply_counts),
                pane_prefix: "timeline",
                scroll_target: pending_scroll_id,
                solo: false,
            },
            channels,
            cx,
        );
        let target_scroll_item_index = target_in_rows.map(|i| timeline_items.len() + i);
        timeline_items.extend(rows);
        // `py-3` の下の余白。
        timeline_items.push(div().h(px(12.0)).flex_none().into_any_element());

        let ch_id_for_join = channel_id.clone();
        let theme = self.state.read(cx).theme();

        // 読み込み中は `MessageSkeleton`、空は `MessageState`。
        let placeholder_kind = timeline_placeholder(visible_roots.is_empty(), synced);
        let placeholder: Option<AnyElement> = match placeholder_kind {
            Placeholder::None => None,
            Placeholder::Skeleton => Some(render_message_skeleton().into_any_element()),
            Placeholder::Empty => Some(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .p(px(24.0))
                    .child(
                        div()
                            .text_size(sm.size())
                            .line_height(sm.line())
                            .font(crate::fonts::sans(500.0))
                            .child(i18n.t("messages.empty")),
                    )
                    .when(can_post_hint(is_member, is_archived), |d| {
                        d.child(
                            div()
                                .text_size(xs.size())
                                .line_height(xs.line())
                                .text_color(t.muted_foreground)
                                .child(i18n.t("messages.first")),
                        )
                    })
                    .into_any_element(),
            ),
        };

        let scroll_area = match placeholder {
            _ if is_encrypted => div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(render_encrypted_unsupported(i18n, &p)),
            Some(p) => div().relative().flex_1().min_h_0().child(p),
            None => div().relative().flex_1().min_h_0().child(
                ScrollArea::new("timeline-scroll-area")
                    .scroll_handle(&self.timeline_scroll)
                    .size_full()
                    .children(timeline_items),
            ),
        };

        let timeline_view = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            // チャンネルヘッダー
            .child({
                let (is_dm, is_group_dm, is_private, is_nsfw) =
                    header_kind.unwrap_or((false, false, false, false));
                self.render_channel_header(
                    ChannelHeaderInfo {
                        title: ch_label,
                        description: ch_desc,
                        member_count: dm_member_count,
                        is_dm,
                        is_group_dm,
                        is_private,
                        is_nsfw,
                        is_archived,
                        failed_draft_count,
                        can_join,
                        can_leave,
                        is_member,
                        channel_id: ch_id_for_join,
                    },
                    cx,
                )
            })
            // 検索バー
            .when(is_searching, |d| {
                d.child(self.render_message_search_bar(cx))
            })
            // 削除確認バー
            .when(confirm_delete_id.is_some(), |d| {
                d.child(self.render_delete_confirm_bar(theme, cx))
            })
            // タイムライン本体
            .child(
                scroll_area
                    // 最新へ戻るボタン (`absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full shadow-md`)
                    .when(
                        !is_encrypted
                            && placeholder_kind == Placeholder::None
                            && (!timeline_at_bottom || hidden_after > 0),
                        |d| {
                            let label = if hidden_after > 0 {
                                format!("{} ({hidden_after})", i18n.t("messages.new"))
                            } else {
                                i18n.t("messages.new")
                            };
                            d.child(
                                h_flex()
                                    .absolute()
                                    .bottom(px(12.0))
                                    .left_0()
                                    .right_0()
                                    .justify_center()
                                    .child(
                                        Button::new("btn-jump-to-latest")
                                            .size(ButtonSize::Sm)
                                            .icon_start(Icon::ArrowDown)
                                            .label(label)
                                            .rounded_px(9999.0)
                                            .shadow(shadow_md())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.jump_to_latest_messages();
                                                    cx.notify();
                                                });
                                            })),
                                    ),
                            )
                        },
                    ),
            )
            // 下部入力欄
            .child(self.render_composer_view(cx));

        (
            timeline_view,
            target_scroll_item_index,
            placeholder_kind != Placeholder::None,
        )
    }
}

/// タイムライン本体に出すもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Placeholder {
    None,
    Skeleton,
    Empty,
}

/// frontend は `items.length === 0` のときだけ placeholder を出す。
/// 同期前でも行があれば出し、同期済みで空なら空表示、同期前で空ならスケルトン。
pub(super) fn timeline_placeholder(visible_is_empty: bool, synced: bool) -> Placeholder {
    match (visible_is_empty, synced) {
        (false, _) => Placeholder::None,
        (true, false) => Placeholder::Skeleton,
        (true, true) => Placeholder::Empty,
    }
}

/// 空のときに「最初のメッセージを送ってみましょう」を添えるか (`canCompose`)。
fn can_post_hint(is_member: bool, is_archived: bool) -> bool {
    is_member && !is_archived
}

/// `MessageSkeleton`。6 行ぶんのアバターと、名前と本文の棒。
pub(super) fn render_message_skeleton() -> Div {
    // `flex flex-col gap-4 p-4`
    let mut list = v_flex().gap(px(16.0)).p(px(16.0));
    for row in 0..6usize {
        list = list.child(
            h_flex()
                .items_start()
                .gap(px(12.0))
                // `size-9 shrink-0 rounded-full`
                .child(
                    Skeleton::new(("msg-skeleton-avatar", row))
                        .size(px(36.0))
                        .flex_none()
                        .rounded_full(),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(8.0))
                        .child(
                            Skeleton::new(("msg-skeleton-name", row))
                                .h(px(12.0))
                                .w(px(112.0)),
                        )
                        .child(
                            Skeleton::new(("msg-skeleton-body", row))
                                .h(px(16.0))
                                .w_full()
                                .max_w(px(448.0)),
                        ),
                ),
        );
    }
    list
}

/// 暗号化チャンネルを開いたときの表示。デスクトップ版は E2EE の対象外なので、本文も入力欄も出さない
/// （docs/e2ee.md の「クライアントの構成」、#361）。
fn render_encrypted_unsupported(i18n: I18n, p: &Palette) -> Div {
    let t = p.t;
    let sm = TextSpec::SM;
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .p(px(24.0))
        .child(icon_colored(Icon::LockKeyhole, 24.0, t.muted_foreground))
        .child(
            div()
                .text_size(TextSpec::BASE.size())
                .line_height(TextSpec::BASE.line())
                .font(crate::fonts::sans(600.0))
                .text_color(t.foreground)
                .child(i18n.t("e2ee.desktopUnsupported.title")),
        )
        .child(
            div()
                .text_size(sm.size())
                .line_height(sm.line())
                .text_color(t.muted_foreground)
                .child(i18n.t("e2ee.desktopUnsupported.description")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_only_when_no_rows() {
        // 行があれば同期前でもスケルトンで隠さない。
        assert_eq!(timeline_placeholder(false, false), Placeholder::None);
        assert_eq!(timeline_placeholder(false, true), Placeholder::None);
        assert_eq!(timeline_placeholder(true, false), Placeholder::Skeleton);
        assert_eq!(timeline_placeholder(true, true), Placeholder::Empty);
    }

    #[test]
    fn empty_hint_only_for_members_of_open_channels() {
        assert!(can_post_hint(true, false));
        assert!(!can_post_hint(false, false));
        assert!(!can_post_hint(true, true));
    }
}
