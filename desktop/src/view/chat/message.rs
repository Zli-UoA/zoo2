//! メッセージ行 (frontend の `MessageRow.tsx` / `MessageList.tsx` の行と区切り)。
//!
//! 写していないもの (デスクトップに元のデータや画面がないため):
//! - 行の「…」メニュー (生の本文のコピー、アプリの操作)。ポップアップの置き場は PR 5 のダイアログと一緒に作る。
//! - リアクションを付けた人の一覧 (ツールチップとダイアログ。`ReactionGroup` に人が入っていない)
//! - 編集中の行の中の入力欄 (編集は今のまま下部の入力欄で行う。行は強調だけする)
//! - ActivityPub のバッジ、アプリのブロックと添付 (`apub` / `app_*` の列を持っていない)
//! - ユーザーのアバター画像 (`avatar_url` の列を持っていない。頭文字か bot のアイコンを出す)
//! - 長押しのメニュー (タッチ端末向け)

use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyElement, Context, ElementId, FocusHandle, Focusable, MouseButton, MouseUpEvent,
    ParentElement, SharedString, Styled, div, prelude::*, px,
};
use zoo2_client_core::{
    electric::rows::{Attachment, Message, parse_message_cards},
    store::ReactionGroup,
};

use crate::app::{QUICK_EMOJIS, Target, reaction_label};
use crate::fonts;
use crate::icons::Icon;
use crate::mfm_view::{self, MfmColors};
use crate::tokens::{Tokens, radius};
use crate::ui::avatar::{Avatar, AvatarSize};
use crate::ui::tooltip::tooltip_view;
use crate::ui::{TextSpec, alpha, h_flex, icon_colored, palette, ring, shadow_sm, v_flex};
use crate::view::chat::attachments::{AttachmentItem, render_message_attachments};
use crate::view::chat::message_format::{
    GROUP_GAP_MS, MessageItem, build_message_items, format_clock, relative_day_label,
};
use crate::view::render_ogp_card;
use crate::view::shell::sidebar::initials;
use crate::view::{WorkspaceView, local_zoned};

/// 行の外側の余白 (`px-2`)。一覧の左右に付く。
pub const LIST_GUTTER: f32 = 8.0;

/// 日付の区切り (`DateSeparator`)。
pub fn render_date_separator(label: &str, t: &Tokens) -> AnyElement {
    // `flex items-center gap-3 py-2`
    h_flex()
        .mx(px(LIST_GUTTER))
        .py(px(8.0))
        .gap(px(12.0))
        .child(div().h(px(1.0)).flex_1().bg(t.border))
        .child(
            // `rounded-full border bg-background/80 px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground`
            div()
                .px(px(10.0))
                .py(px(2.0))
                .rounded_full()
                .border_1()
                .border_color(t.border)
                .bg(alpha(t.background, 0.8))
                .text_size(px(11.0))
                .line_height(px(16.5))
                .font(fonts::sans(500.0))
                .text_color(t.muted_foreground)
                .child(label.to_string()),
        )
        .child(div().h(px(1.0)).flex_1().bg(t.border))
        .into_any_element()
}

/// 未読の線 (`messages.new` の区切り)。
pub fn render_unread_line(label: &str, t: &Tokens) -> AnyElement {
    // `flex items-center gap-2 py-2 text-xs font-semibold text-destructive`
    let xs = TextSpec::XS;
    h_flex()
        .mx(px(LIST_GUTTER))
        .py(px(8.0))
        .gap(px(8.0))
        .text_size(xs.size())
        .line_height(xs.line())
        .font(fonts::sans(600.0))
        .text_color(t.destructive)
        .child(div().h(px(1.0)).flex_1().bg(t.destructive))
        .child(label.to_string())
        .child(div().h(px(1.0)).w(px(24.0)).bg(t.destructive))
        .into_any_element()
}

/// ホバーのツールバーのアイコンボタン (`Button variant="ghost" size="icon-xs"`)。
/// 削除だけは `text-destructive hover:bg-destructive/10`。
/// キーボードでも押せるように、フォーカスとクリックを同じ要素に付ける (Enter / Space で押せる)。
#[allow(clippy::too_many_arguments)]
fn toolbar_button(
    id: String,
    icon: Icon,
    label: String,
    destructive: bool,
    focus: &FocusHandle,
    focused: bool,
    t: &Tokens,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (fg, hover_bg) = if destructive {
        (t.destructive, alpha(t.destructive, 0.1))
    } else {
        (t.foreground, t.muted)
    };
    div()
        .id(ElementId::Name(SharedString::from(id)))
        .track_focus(focus)
        // `focus-visible:ring-[3px] focus-visible:ring-ring/50`
        .when(focused, |d| {
            d.bg(hover_bg).shadow(vec![ring(alpha(t.ring, 0.5), 3.0)])
        })
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(24.0))
        .rounded(px(f32::from(radius::MD).min(10.0)))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .tooltip(tooltip_view(label))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx)
        })
        .child(icon_colored(icon, 16.0, fg))
        .into_any_element()
}

/// 一覧の行をまとめて作るときの、一覧ごとに決まる入力。
pub(super) struct RowList<'a> {
    pub messages: Vec<&'a Message>,
    /// 未読の線を置くメッセージ。
    pub first_unread_id: Option<&'a str>,
    pub me_id: &'a str,
    pub me_user_name: &'a str,
    pub is_archived: bool,
    pub reactions: &'a HashMap<String, Vec<ReactionGroup>>,
    pub attachments: &'a HashMap<String, Vec<Attachment>>,
    /// スレッドのピルを出す一覧 (タイムライン) だけ渡す。
    pub reply_counts: Option<&'a HashMap<String, usize>>,
    pub pane_prefix: &'a str,
    /// スクロールで合わせるメッセージ。
    pub scroll_target: Option<&'a str>,
    /// 連投のまとめも日付の区切りもなく、1 行ずつ単独で出す (スレッドの親投稿)。
    pub solo: bool,
}

impl WorkspaceView {
    /// `#名前` → チャンネル ID の対応 (DM を除く)。描画ごとに 1 回だけ作って各一覧に渡す。
    pub(in crate::view) fn channel_links(&self, cx: &Context<Self>) -> Rc<HashMap<String, String>> {
        Rc::new(
            self.state
                .read(cx)
                .store
                .channels()
                .into_iter()
                .filter(|c| !c.is_dm)
                .map(|c| (c.channel_name.clone(), c.id.clone()))
                .collect(),
        )
    }

    /// 日付の区切り、未読の線、行を並べる (`MessageList` の行)。
    /// 返す 2 つ目は、`scroll_target` の行の位置 (返す列の中の番号)。
    pub(super) fn render_message_rows(
        &self,
        list: RowList<'_>,
        channels: Rc<HashMap<String, String>>,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Option<usize>) {
        let t = palette(cx).t;
        let i18n = self.i18n;
        // `#名前` は開けるチャンネルへのリンクにする (`MfmChannelsProvider`)。
        let mut colors = MfmColors::from_tokens(&t);
        colors.channels = Some(channels);
        let state_entity = self.state.clone();
        colors.on_channel = Some(Rc::new(move |id: &str, cx: &mut gpui::App| {
            let id = id.to_string();
            state_entity.update(cx, |s, cx| {
                s.open_channel(id);
                cx.notify();
            });
        }));
        let to_local = |d| local_zoned(d).map(|z| z.datetime()).unwrap_or(d);
        let items = if list.solo {
            vec![MessageItem::SOLO; list.messages.len()]
        } else {
            build_message_items(&list.messages, GROUP_GAP_MS, list.first_unread_id, to_local)
        };
        let today = jiff::Zoned::now().date();
        let mut out = Vec::new();
        let mut target = None;
        for (message, item) in list.messages.iter().zip(items) {
            // 未読の線は、行の中の日付の区切りより上に出す (`MessageList` と `MessageRow` の順)。
            if list.first_unread_id == Some(message.id.as_str()) {
                out.push(render_unread_line(&i18n.t("messages.new"), &t));
            }
            if item.show_date_separator {
                let day = to_local(message.created_at).date();
                out.push(render_date_separator(
                    &relative_day_label(day, today, &i18n),
                    &t,
                ));
            }
            if list.scroll_target == Some(message.id.as_str()) {
                target = Some(out.len());
            }
            let reactions = list.reactions.get(&message.id).cloned().unwrap_or_default();
            let attachments = list
                .attachments
                .get(&message.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let reply_count = list
                .reply_counts
                .map(|counts| counts.get(&message.id).copied().unwrap_or(0));
            out.push(self.render_message_row(
                message,
                item,
                &colors,
                list.me_id,
                list.me_user_name,
                list.is_archived,
                reactions,
                attachments,
                reply_count,
                list.pane_prefix,
                cx,
            ));
        }
        (out, target)
    }

    /// 1 行ぶんのメッセージ。区切りは呼び出し側が前に置く。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_message_row(
        &self,
        message: &Message,
        item: MessageItem,
        colors: &MfmColors,
        me_id: &str,
        me_user_name: &str,
        is_archived: bool,
        reactions: Vec<ReactionGroup>,
        attachments: &[Attachment],
        reply_count: Option<usize>,
        pane_prefix: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        let xs = TextSpec::XS;
        let state = self.state.read(cx);
        let msg_id = message.id.clone();
        let body = mfm_view::render_mfm_accounts_with_colors(
            &format!("{pane_prefix}-{msg_id}"),
            &message.content,
            Some(me_user_name),
            colors,
            &state.store.account_profiles,
        );
        let author_name = state.store.display_name(&message.user_id);
        let is_bot = state.store.user(&message.user_id).is_some_and(|u| u.is_bot);
        let is_mine = message.user_id == me_id;
        let is_edited = message.updated_at.is_some();
        let is_editing = matches!(&state.target, Target::Edit(id) if id == &msg_id);
        let time_str = local_zoned(message.created_at)
            .map(|z| format_clock(z.datetime()))
            .unwrap_or_default();
        let reaction_picker_open = !is_archived
            && state
                .active_reaction_message
                .as_ref()
                .is_some_and(|(pane, id)| pane == pane_prefix && id == &msg_id);
        let legacy_theme = state.theme();

        let is_search_matched = state
            .message_search
            .as_ref()
            .is_some_and(|ms| ms.matches.contains(&msg_id));
        let is_current_search_match = state
            .message_search
            .as_ref()
            .and_then(|ms| ms.matches.get(ms.current_match_index))
            == Some(&msg_id);

        let attachment_items: Vec<AttachmentItem> = attachments
            .iter()
            .map(|att| AttachmentItem {
                attachment: att.clone(),
                download_url: state.attachment_download_url(&att.id),
                is_fetching: state.fetching_attachment_urls.contains(&att.id)
                    || state.pending_open_attachments.contains(&att.id),
                needs_refresh: state.attachment_url_needs_refresh(&att.id),
                is_failed: state.failed_attachment_urls.contains_key(&att.id),
            })
            .collect();

        // 連携カード (`card` 列) があれば、本文の代わりにカードを出す (`MessageBody`)。
        let cards = message
            .card
            .as_deref()
            .map(parse_message_cards)
            .unwrap_or_default();
        let card_views: Vec<AnyElement> = cards
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
                    &i18n,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let attachments_view =
            render_message_attachments(attachment_items, &self.attachment_focus_handles, i18n, cx);

        let group = SharedString::from(format!("row-{pane_prefix}-{msg_id}"));
        let row_id = ElementId::Name(SharedString::from(format!("{pane_prefix}-row-{msg_id}")));

        // --- 左の列 (アバター、または連投の行の時刻) ---
        let left = if item.is_group_start {
            let avatar = Avatar::new().size(AvatarSize::Default);
            let avatar = if is_bot {
                avatar.icon(Icon::Bot)
            } else {
                avatar.initials(initials(&author_name))
            };
            // `mt-0.5`
            div().mt(px(2.0)).flex_none().child(avatar)
        } else {
            // `flex h-lh w-8 shrink-0 items-center justify-end text-sm leading-relaxed`
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .w(px(32.0))
                .h(px(22.75))
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(10.0))
                        .text_color(t.muted_foreground)
                        .opacity(0.0)
                        .group_hover(group.clone(), |s| s.opacity(1.0))
                        .child(time_str.clone()),
                )
        };

        // 編集済みの印 (`EditedIndicator`)。グループの先頭では名前の行に、連投の行では本文の後ろに置く。
        let edited_marker = || {
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "{pane_prefix}-edited-{msg_id}"
                ))))
                .flex_none()
                .tooltip(tooltip_view(i18n.t("messages.edited")))
                .child(icon_colored(
                    Icon::Pencil,
                    12.0,
                    alpha(t.muted_foreground, 0.8),
                ))
        };
        let edited_in_header = (is_edited && item.is_group_start).then(edited_marker);
        let edited_after_body = (is_edited && !item.is_group_start).then(edited_marker);
        // カードの行は本文を出さないので、カードの後ろに置く。
        let (edited_after_body, edited_after_cards) = if cards.is_empty() {
            (edited_after_body, None)
        } else {
            (None, edited_after_body)
        };

        // --- 名前の行 ---
        let header = item.is_group_start.then(|| {
            // `flex flex-wrap items-center gap-2`
            h_flex()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    // `inline-flex min-w-0 items-center gap-1.5`
                    h_flex()
                        .min_w_0()
                        .gap(px(6.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(sm.size())
                                .line_height(sm.line())
                                .font(fonts::sans(600.0))
                                .child(author_name.clone()),
                        )
                        .when(is_bot, |d| {
                            // `rounded bg-muted px-1 py-0.5 text-[9px] font-semibold text-muted-foreground`
                            d.child(
                                div()
                                    .flex_none()
                                    .px(px(4.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(t.muted)
                                    .text_size(px(9.0))
                                    .line_height(px(13.5))
                                    .font(fonts::sans(600.0))
                                    .text_color(t.muted_foreground)
                                    .child("APP"),
                            )
                        }),
                )
                .child(
                    // `shrink-0 text-[11px] text-muted-foreground tabular-nums`
                    div()
                        .flex_none()
                        .text_size(px(11.0))
                        .line_height(px(16.5))
                        .text_color(t.muted_foreground)
                        .child(time_str.clone()),
                )
                .children(edited_in_header)
        });

        // --- スレッドのピル (`ReplyLink`) ---
        let msg_id_thread = msg_id.clone();
        let reply_pill = reply_count.filter(|&c| c > 0).map(|count| {
            let label = i18n.t_count("threads.replyCount", count as i64, &[]);
            // `mt-1 flex w-fit items-center gap-1 rounded-full border border-brand/30 bg-brand-muted/50 px-2 py-0.5 text-xs font-medium text-brand-muted-foreground hover:bg-brand-muted`
            h_flex().mt(px(4.0)).child(
                h_flex()
                    .id(ElementId::Name(SharedString::from(format!(
                        "btn-open-thread-{}",
                        message.id
                    ))))
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded_full()
                    .border_1()
                    .border_color(alpha(t.brand, 0.3))
                    .bg(alpha(t.brand_muted, 0.5))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .font(fonts::sans(500.0))
                    .text_color(t.brand_muted_foreground)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.brand_muted))
                    .child(icon_colored(
                        Icon::MessageSquareText,
                        14.0,
                        t.brand_muted_foreground,
                    ))
                    .child(label)
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
        });

        // --- リアクションのピッカーを開閉する (チップの「+」とツールバーで共有) ---
        let open_reaction_picker = {
            let msg_id = msg_id.clone();
            let pane = pane_prefix.to_string();
            Rc::new(cx.listener(
                move |this: &mut Self, _: &(), window: &mut gpui::Window, cx| {
                    let target_key = (pane.clone(), msg_id.clone());
                    let opened = this.state.update(cx, |s, cx| {
                        if s.active_reaction_message.as_ref() == Some(&target_key) {
                            s.active_reaction_message = None;
                            cx.notify();
                            false
                        } else {
                            s.active_reaction_message = Some(target_key);
                            cx.notify();
                            true
                        }
                    });
                    if opened {
                        this.reaction_input.update(cx, |input, cx| input.clear(cx));
                        window.focus(&this.reaction_input.focus_handle(cx));
                    }
                },
            ))
        };

        // --- リアクションのチップ ---
        let mut chips = Vec::new();
        for reaction in &reactions {
            let emoji = reaction.emoji.clone();
            let msg_for_toggle = msg_id.clone();
            let mine = reaction.mine;
            // `inline-flex min-h-8 items-center gap-1 rounded-full border px-2 text-xs hover:bg-muted`
            // 自分のものは `border-brand/40 bg-brand-muted text-brand-muted-foreground`
            chips.push(
                h_flex()
                    .id(ElementId::Name(SharedString::from(format!(
                        "{pane_prefix}-react-pill-{}-{emoji}",
                        message.id
                    ))))
                    .min_h(px(32.0))
                    .gap(px(4.0))
                    .px(px(8.0))
                    .rounded_full()
                    .border_1()
                    .border_color(if mine { alpha(t.brand, 0.4) } else { t.border })
                    .when(mine, |d| d.bg(t.brand_muted))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .text_color(if mine {
                        t.brand_muted_foreground
                    } else {
                        t.foreground
                    })
                    .when(!is_archived, |d| d.cursor_pointer())
                    .hover(|s| s.bg(t.muted))
                    .child(
                        div()
                            .text_size(px(20.0))
                            .line_height(px(20.0))
                            .child(reaction_label(&reaction.emoji)),
                    )
                    .child(div().child(reaction.count.to_string()))
                    .when(!is_archived, |d| {
                        d.on_mouse_up(
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
                    }),
            );
        }
        // ツールバーのボタンのフォーカス。描画をまたいで同じものを使う。
        let row_key = format!("{pane_prefix}:{msg_id}");
        let toolbar_focus = |action: &str, cx: &mut Context<Self>| -> (FocusHandle, bool) {
            let key = format!("{row_key}:{action}");
            let focused = self.focused_toolbar_key.as_deref() == Some(key.as_str());
            let handle = self
                .toolbar_focus_handles
                .borrow_mut()
                .entry(key)
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            (handle, focused)
        };
        let toolbar_row_focused = self
            .focused_toolbar_key
            .as_deref()
            .and_then(|k| k.rsplit_once(':'))
            .is_some_and(|(row, _)| row == row_key);
        let reactions_view = (!chips.is_empty()).then(|| {
            // `flex flex-wrap items-center gap-1 py-1`
            h_flex()
                .flex_wrap()
                .gap(px(4.0))
                .py(px(4.0))
                .children(chips)
                .when(!is_archived, |d| {
                    let open = open_reaction_picker.clone();
                    let (focus, focused) = toolbar_focus("react-add", cx);
                    d.child(toolbar_button(
                        format!("{pane_prefix}-btn-react-add-{msg_id}"),
                        Icon::SmilePlus,
                        i18n.t("reactions.add"),
                        false,
                        &focus,
                        focused,
                        &t,
                        move |window, cx| open(&(), window, cx),
                    ))
                })
        });

        // --- 絵文字のクイック選択 (入力欄つき。見た目は PR 5 で作り直す) ---
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
                        .bg(legacy_theme.bg_primary)
                        .border_1()
                        .border_color(legacy_theme.border)
                        .hover(|s| s.bg(legacy_theme.bg_hover).cursor_pointer())
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

        // --- ホバーのツールバー ---
        let toolbar = (!is_archived && !is_editing).then(|| {
            let mut buttons: Vec<AnyElement> = Vec::new();
            let open = open_reaction_picker.clone();
            let (focus, focused) = toolbar_focus("react", cx);
            buttons.push(toolbar_button(
                format!("{pane_prefix}-btn-react-{msg_id}"),
                Icon::SmilePlus,
                i18n.t("reactions.add"),
                false,
                &focus,
                focused,
                &t,
                move |window, cx| open(&(), window, cx),
            ));
            {
                // 返信ボタン。スレッドを開いて入力欄を返信先にする。
                // frontend の `ThreadPane` も行に `onOpenThread` を渡すので、スレッドの行にも出す。
                let view = cx.entity().clone();
                // スレッドの返信の行では、返信の ID ではなく親 (スレッドのルート) を開く。
                let root = message.parent_id.clone().unwrap_or_else(|| msg_id.clone());
                let (focus, focused) = toolbar_focus("reply", cx);
                buttons.push(toolbar_button(
                    format!("{pane_prefix}-btn-reply-{msg_id}"),
                    Icon::MessageSquarePlus,
                    i18n.t("threads.open"),
                    false,
                    &focus,
                    focused,
                    &t,
                    move |window, cx| {
                        view.update(cx, |this, cx| {
                            let r_id = root.clone();
                            this.state.update(cx, |s, cx| {
                                s.open_thread(r_id);
                                cx.notify();
                            });
                            window.focus(&this.composer_input.focus_handle(cx));
                        });
                    },
                ));
            }
            if is_mine {
                let view = cx.entity().clone();
                let id = msg_id.clone();
                let (focus, focused) = toolbar_focus("edit", cx);
                buttons.push(toolbar_button(
                    format!("{pane_prefix}-btn-edit-{msg_id}"),
                    Icon::Pencil,
                    i18n.t("messages.edit"),
                    false,
                    &focus,
                    focused,
                    &t,
                    move |window, cx| {
                        view.update(cx, |this, cx| {
                            let m_id = id.clone();
                            this.state.update(cx, |s, cx| {
                                s.start_edit(m_id);
                                cx.notify();
                            });
                            window.focus(&this.composer_input.focus_handle(cx));
                        });
                    },
                ));
                let view = cx.entity().clone();
                let id = msg_id.clone();
                let (focus, focused) = toolbar_focus("delete", cx);
                buttons.push(toolbar_button(
                    format!("{pane_prefix}-btn-delete-{msg_id}"),
                    Icon::Trash,
                    i18n.t("messages.delete"),
                    true,
                    &focus,
                    focused,
                    &t,
                    move |_, cx| {
                        view.update(cx, |this, cx| {
                            let m_id = id.clone();
                            this.state.update(cx, |s, cx| {
                                s.request_delete(m_id);
                                cx.notify();
                            });
                        });
                    },
                ));
            }
            // `absolute right-1.5 top-0 flex items-center gap-0.5 rounded-lg border bg-popover p-0.5 opacity-0 shadow-sm group-hover/row:opacity-100`
            h_flex()
                .absolute()
                .right(px(6.0))
                .top_0()
                .gap(px(2.0))
                .p(px(2.0))
                .rounded(radius::LG)
                .border_1()
                .border_color(t.border)
                .bg(t.popover)
                .shadow(shadow_sm())
                .when(!toolbar_row_focused, |d| {
                    d.opacity(0.0)
                        .group_hover(group.clone(), |s| s.opacity(1.0))
                })
                .children(buttons)
        });

        // --- 行 ---
        let mut row = div()
            .id(row_id)
            .group(group.clone())
            .relative()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.0))
            .mx(px(LIST_GUTTER))
            .px(px(12.0))
            .pt(px(if item.is_group_start { 6.0 } else { 0.0 }))
            .pb(px(if item.is_group_end { 6.0 } else { 0.0 }))
            .rounded(radius::LG)
            .hover(|s| s.bg(alpha(t.muted, 0.6)))
            .child(left);
        if is_editing {
            row = row.bg(alpha(t.muted, 0.6));
        }
        if is_current_search_match || is_search_matched {
            // 対象のメッセージと同じ `bg-brand-muted/40`。いまの一致には `ring-1 ring-brand` も付ける。
            row = row.bg(alpha(t.brand_muted, 0.4));
            if is_current_search_match {
                row = row.shadow(vec![ring(t.brand, 1.0)]);
            }
        }

        row.child(
            // `min-w-0 flex-1`
            v_flex()
                .flex_1()
                .min_w_0()
                .children(header)
                .when(cards.is_empty(), |d| match edited_after_body {
                    Some(marker) => d.child(
                        h_flex()
                            .items_start()
                            .gap(px(4.0))
                            .child(div().flex_1().min_w_0().child(body))
                            .child(div().mt(px(5.0)).child(marker)),
                    ),
                    None => d.child(body),
                })
                .children(card_views)
                .children(edited_after_cards)
                .children(attachments_view)
                .children(reply_pill)
                .children(reactions_view)
                // インラインリアクションピッカー
                .when(reaction_picker_open, |d| {
                    d.child(
                        div()
                            .mt_1()
                            .p_2()
                            .rounded_md()
                            .bg(legacy_theme.bg_secondary)
                            .border_1()
                            .border_color(legacy_theme.border_accent)
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
                }),
        )
        .children(toolbar)
        .into_any_element()
    }
}
