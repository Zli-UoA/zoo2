//! チャンネルサイドバー (frontend の `ChannelSidebar` / `DmSection` / `DmListItem` /
//! `sidebarStyles.ts` に対応)。
//!
//! 写していないもの (デスクトップに元のデータや画面がないため):
//! - `ChannelThreadItems` (スレッドの索引 `useThreadIndex` に当たるデータがない)
//! - 通話中の表示 (`call.activeCalls` に当たるデータがない)
//! - DM セクションの「+」(DM の作成画面がない)
//! - 検索ボタンの行き先は「チャンネルを探す」ではなく、既存のクイックスイッチャー (⌘K)

use gpui::{
    AnyElement, Context, Div, ElementId, MouseButton, MouseUpEvent, ParentElement, Rgba,
    SharedString, Styled, div, prelude::*, px,
};
use zoo2_client_core::electric::ShapeId;
use zoo2_client_core::electric::rows::Channel;

use crate::app::AppState;
use crate::fonts;
use crate::i18n::I18n;
use crate::icons::Icon;
use crate::tokens::{Tokens, radius};
use crate::ui::avatar::{Avatar, AvatarSize};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::collapsible_section::CollapsibleSection;
use crate::ui::scroll_area::ScrollArea;
use crate::ui::skeleton::Skeleton;
use crate::ui::{TRANSPARENT, TextSpec, alpha, h_flex, icon_colored, palette, v_flex};
use crate::view::WorkspaceView;

/// サイドバーの幅 (`md:w-72`)。
pub const SIDEBAR_WIDTH: f32 = 288.0;
/// 行の高さ (`md:h-8`)。
pub const ROW_HEIGHT: f32 = 32.0;
/// 未読の点の直径 (`size-2`)。
pub const UNREAD_DOT: f32 = 8.0;

/// 行の見た目 (`sidebarItemClassName(active, unread)` の結果)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowStyle {
    pub bg: Rgba,
    pub fg: Rgba,
    pub bold: bool,
    pub hover_bg: Rgba,
    pub hover_fg: Rgba,
}

/// `sidebarItemClassName`。`dimmed` は参加していないチャンネル (Web では一覧に出ない) の文字を薄くする。
pub fn row_style(t: &Tokens, active: bool, unread: bool, dimmed: bool) -> RowStyle {
    if active {
        // `bg-brand-muted font-semibold text-brand-muted-foreground` (hover で変わらない)
        return RowStyle {
            bg: t.brand_muted,
            fg: t.brand_muted_foreground,
            bold: true,
            hover_bg: t.brand_muted,
            hover_fg: t.brand_muted_foreground,
        };
    }
    if unread {
        // `font-semibold text-sidebar-foreground hover:bg-sidebar-accent`
        return RowStyle {
            bg: TRANSPARENT,
            fg: t.sidebar_foreground,
            bold: true,
            hover_bg: t.sidebar_accent,
            hover_fg: t.sidebar_foreground,
        };
    }
    // `text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground`
    RowStyle {
        bg: TRANSPARENT,
        fg: if dimmed {
            alpha(t.muted_foreground, 0.6)
        } else {
            t.muted_foreground
        },
        bold: false,
        hover_bg: t.sidebar_accent,
        hover_fg: t.sidebar_accent_foreground,
    }
}

/// グループ DM か (`dm.ts` の `isGroupDm`)。
pub fn is_group_dm(channel_name: &str) -> bool {
    channel_name.starts_with("dm:group:")
}

/// DM の表示名 (`dm.ts` の `resolveDmDisplayName` と `formatDmDisplayName`)。
/// 名前を並べ替え、先頭の `max` 人だけを出し、残りは `more(残りの人数)` で足す。
pub fn dm_display_name(
    mut names: Vec<String>,
    max: usize,
    untitled: &str,
    more: impl Fn(usize) -> String,
) -> String {
    names.retain(|n| !n.is_empty());
    // `Intl.Collator` の近似。大文字小文字を無視して並べ、同じなら元の文字列で決める。
    names.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    let max = max.max(1);
    let remaining = names.len().saturating_sub(max);
    names.truncate(max);
    let joined = if names.is_empty() {
        untitled.to_string()
    } else {
        names.join(", ")
    };
    if remaining > 0 {
        format!("{joined} {}", more(remaining))
    } else {
        joined
    }
}

/// `format.ts` の `initials`。
pub fn initials(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "?".to_string();
    }
    let letters: String = trimmed
        .split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters
    }
}

/// 1 対 1 の DM の名前 (`dm:<id>:<id>` または `edm:<id>:<id>`) から 2 人の ID を取り出す (`parseOneToOneDmIds`)。
pub fn parse_one_to_one_dm_ids(channel_name: &str) -> Option<(&str, &str)> {
    if is_group_dm(channel_name) {
        return None;
    }
    let rest = channel_name
        .strip_prefix("edm:")
        .or_else(|| channel_name.strip_prefix("dm:"))?;
    let (a, b) = rest.split_once(':')?;
    if a.is_empty() || b.is_empty() || b.contains(':') {
        return None;
    }
    Some((a, b))
}

/// DM の参加者の ID。1 対 1 は名前から、グループはメンバー行から (`resolveDmDisplayName`)。
/// 自分は除く。
pub fn dm_other_ids(channel_name: &str, member_ids: &[String], me_id: &str) -> Vec<String> {
    let all: Vec<String> = match parse_one_to_one_dm_ids(channel_name) {
        Some((a, b)) => vec![a.to_string(), b.to_string()],
        None => member_ids.to_vec(),
    };
    if me_id.is_empty() {
        all
    } else {
        all.into_iter().filter(|id| id != me_id).collect()
    }
}

/// 一覧とヘッダーに出す名前。DM は自分以外の参加者の名前 (`formatDmDisplayName`)、通常のチャンネルは名前そのまま。
/// 1 対 1 の DM はメンバー行が消えても名前の ID から引ける。名前を引けないユーザーは出さない。
pub(in crate::view) fn channel_title(
    state: &AppState,
    ch: &Channel,
    me_id: &str,
    i18n: &I18n,
) -> String {
    if !ch.is_dm {
        return ch.channel_name.clone();
    }
    let member_ids: Vec<String> = if parse_one_to_one_dm_ids(&ch.channel_name).is_some() {
        Vec::new()
    } else {
        state
            .store
            .channel_members(&ch.id)
            .into_iter()
            .map(|u| u.id.clone())
            .collect()
    };
    let names = dm_other_ids(&ch.channel_name, &member_ids, me_id)
        .into_iter()
        .filter(|id| state.store.user(id).is_some())
        .map(|id| state.store.display_name(&id))
        .collect();
    dm_display_name(names, 3, &i18n.t("dms.untitled"), |n| {
        i18n.t_count("dms.moreNames", n as i64, &[])
    })
}

/// 一覧の 1 行ぶんのデータ。描画の前に `AppState` から取り出す。
struct Row {
    id: String,
    label: String,
    is_dm: bool,
    is_group: bool,
    is_private: bool,
    is_nsfw: bool,
    is_archived: bool,
    active: bool,
    unread: bool,
    member: bool,
}

impl WorkspaceView {
    pub(in crate::view) fn render_sidebar_view(&self, cx: &mut Context<Self>) -> Div {
        let i18n = self.i18n;
        let state = self.state.read(cx);
        let p = palette(cx);
        let t = p.t;
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let channels_synced = state.store.is_synced(&ShapeId::Channels);
        let sync_error = state.sync_error();
        let notice = state.notice.as_ref().map(|(m, _)| m.clone());
        let current_channel = state.current_channel.clone();
        let is_guest = state.is_current_user_guest();

        let mut channel_rows: Vec<Row> = Vec::new();
        let mut dm_rows: Vec<Row> = Vec::new();
        for ch in state.store.channels() {
            let label = channel_title(state, ch, &me_id, &i18n);
            let row = Row {
                id: ch.id.clone(),
                label,
                is_dm: ch.is_dm,
                is_group: ch.is_dm && is_group_dm(&ch.channel_name),
                is_private: ch.is_private,
                is_nsfw: ch.is_nsfw,
                is_archived: ch.archive_at.is_some(),
                active: current_channel.as_deref() == Some(ch.id.as_str()),
                unread: state.unread.contains(&ch.id),
                member: state.store.is_member(&ch.id, &me_id),
            };
            if ch.is_dm {
                dm_rows.push(row);
            } else {
                channel_rows.push(row);
            }
        }

        let dm_count = dm_rows.len();
        let loading = !channels_synced && sync_error.is_none();

        let rows: Vec<AnyElement> = channel_rows
            .iter()
            .map(|row| self.render_sidebar_row(row, cx))
            .collect();
        let dm_elems: Vec<AnyElement> = dm_rows
            .iter()
            .map(|row| self.render_sidebar_row(row, cx))
            .collect();

        // `flex flex-col gap-0.5 px-2 pb-2`
        let list = |children: Vec<AnyElement>| {
            v_flex()
                .gap(px(2.0))
                .px(px(8.0))
                .pb(px(8.0))
                .children(children)
        };
        let skeleton_rows = |prefix: &'static str, n: usize| -> Vec<AnyElement> {
            (0..n)
                .map(|i| {
                    Skeleton::new(ElementId::Name(SharedString::from(format!("{prefix}-{i}"))))
                        .h(px(ROW_HEIGHT))
                        .w_full()
                        .rounded(radius::MD)
                        .into_any_element()
                })
                .collect()
        };
        let text_sm = TextSpec::SM;
        let text_xs = TextSpec::XS;

        // 同期エラーと再試行は、チャンネルの欄を閉じていても見えるように、折りたたみの外に置く。
        let sync_error_block: Option<AnyElement> = if loading {
            None
        } else {
            sync_error.map(|err| {
                v_flex()
                    .items_start()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .child(
                        div()
                            .text_size(text_xs.size())
                            .line_height(text_xs.line())
                            .text_color(t.destructive)
                            .child(err),
                    )
                    .child(
                        h_flex().child(
                            Button::new("btn-sidebar-retry-sync")
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .label(i18n.t("offline.retry"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.retry_sync();
                                        cx.notify();
                                    });
                                })),
                        ),
                    )
                    .into_any_element()
            })
        };

        let channels_body: Vec<AnyElement> = if loading {
            skeleton_rows("sidebar-skel-ch", 5)
        } else {
            let mut body: Vec<AnyElement> = Vec::new();
            if current_channel.is_none()
                && let Some(msg) = notice
            {
                body.push(
                    div()
                        .px(px(8.0))
                        .py(px(4.0))
                        .text_size(text_xs.size())
                        .line_height(text_xs.line())
                        .text_color(t.muted_foreground)
                        .child(msg)
                        .into_any_element(),
                );
            }
            if rows.is_empty() {
                body.push(
                    v_flex()
                        .items_start()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(8.0))
                        .child(
                            div()
                                .text_size(text_sm.size())
                                .line_height(text_sm.line())
                                .text_color(t.muted_foreground)
                                .child(i18n.t("channels.empty")),
                        )
                        .when(!is_guest, |d| {
                            d.child(
                                h_flex().child(
                                    Button::new("btn-sidebar-create-empty")
                                        .variant(ButtonVariant::Outline)
                                        .size(ButtonSize::Sm)
                                        .icon_start(Icon::Plus)
                                        .label(i18n.t("channels.createAction"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_create_channel_modal(window, cx);
                                        })),
                                ),
                            )
                        })
                        .into_any_element(),
                );
            } else {
                body.extend(rows);
            }
            body
        };

        let dms_body: Vec<AnyElement> = if loading {
            skeleton_rows("sidebar-skel-dm", 3)
        } else if dm_elems.is_empty() {
            vec![
                v_flex()
                    .items_start()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(8.0))
                    .child(
                        div()
                            .text_size(text_sm.size())
                            .line_height(text_sm.line())
                            .text_color(t.muted_foreground)
                            .child(i18n.t("dms.empty")),
                    )
                    .into_any_element(),
            ]
        } else {
            dm_elems
        };

        let channels_section = CollapsibleSection::new(
            "sidebar-section-channels",
            i18n.t("channels.title"),
            Icon::Hash,
        )
        .open(self.sidebar_channels_open)
        .on_toggle({
            let on = cx.listener(|this, open: &bool, _, cx| {
                this.sidebar_channels_open = *open;
                cx.notify();
            });
            move |open, w, app| on(&open, w, app)
        })
        .action(
            Button::new("btn-open-quick-switcher")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconSm)
                .icon_start(Icon::Search)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_quick_switcher(window, cx);
                })),
        )
        .when(!is_guest, |s| {
            s.action(
                Button::new("btn-open-create-channel")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon_start(Icon::Plus)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_create_channel_modal(window, cx);
                    })),
            )
        })
        .child(list(channels_body));

        let dms_section =
            CollapsibleSection::new("sidebar-section-dms", i18n.t("dms.title"), Icon::Users)
                .count(if loading {
                    "…".to_string()
                } else {
                    dm_count.to_string()
                })
                .open(self.sidebar_dms_open)
                .on_toggle({
                    let on = cx.listener(|this, open: &bool, _, cx| {
                        this.sidebar_dms_open = *open;
                        cx.notify();
                    });
                    move |open, w, app| on(&open, w, app)
                })
                .child(list(dms_body));

        // `min-w-0 flex-1 flex-col border-r bg-sidebar text-sidebar-foreground md:w-72 md:flex-none`
        v_flex()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .min_w_0()
            .bg(t.sidebar)
            .text_color(t.sidebar_foreground)
            .border_r_1()
            .border_color(t.border)
            .child(
                // `min-h-0 flex-1 py-1`
                ScrollArea::new("sidebar-scroll-list")
                    .scroll_handle(&self.sidebar_scroll)
                    .flex_1()
                    .min_h_0()
                    .py(px(4.0))
                    .children(sync_error_block)
                    .child(channels_section)
                    .child(dms_section),
            )
    }

    fn render_sidebar_row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let style = row_style(&t, row.active, row.unread, !row.is_dm && !row.member);
        let sm = TextSpec::SM;
        let ch_id = row.id.clone();
        let hover_bg = style.hover_bg;
        let hover_fg = style.hover_fg;

        let leading: AnyElement = if row.is_dm {
            if row.is_group {
                // `flex size-6 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground`
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.0))
                    .rounded_full()
                    .bg(t.muted)
                    .child(icon_colored(Icon::Users, 14.0, t.muted_foreground))
                    .into_any_element()
            } else {
                Avatar::new()
                    .size(AvatarSize::Sm)
                    .initials(initials(&row.label))
                    .into_any_element()
            }
        } else if row.is_private {
            icon_colored(Icon::Lock, 16.0, style.fg).into_any_element()
        } else {
            // `opacity-60`
            icon_colored(Icon::Hash, 16.0, alpha(style.fg, 0.6)).into_any_element()
        };

        div()
            .id(ElementId::Name(SharedString::from(format!(
                "sidebar-ch-{ch_id}"
            ))))
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .min_w_0()
            .gap(px(8.0))
            .px(px(8.0))
            .rounded(radius::MD)
            .bg(style.bg)
            .text_color(style.fg)
            .text_size(sm.size())
            .line_height(sm.line())
            .font(fonts::sans(if style.bold { 600.0 } else { 400.0 }))
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg).cursor_pointer())
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                    let id = ch_id.clone();
                    this.state.update(cx, |s, cx| {
                        s.open_channel(id);
                        cx.notify();
                    });
                }),
            )
            .child(leading)
            .child(div().flex_1().min_w_0().truncate().child(row.label.clone()))
            .when(row.is_nsfw, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(TextSpec::XS.size())
                        .line_height(TextSpec::XS.line())
                        .text_color(t.muted_foreground)
                        .child("NSFW"),
                )
            })
            .when(row.is_archived && !row.is_dm, |d| {
                // `rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground`
                d.child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(t.muted)
                        .text_size(px(10.4))
                        .line_height(px(14.0))
                        .font(fonts::sans(400.0))
                        .text_color(t.muted_foreground)
                        .child(i18n.t("moderation.archived")),
                )
            })
            .when(row.unread, |d| {
                // `size-2 shrink-0 rounded-full bg-brand`
                d.child(
                    div()
                        .flex_none()
                        .size(px(UNREAD_DOT))
                        .rounded_full()
                        .bg(t.brand),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::ThemeMode;

    fn t() -> Tokens {
        ThemeMode::Light.tokens()
    }

    #[test]
    fn active_row_uses_brand_muted_and_is_bold() {
        let s = row_style(&t(), true, true, false);
        assert_eq!(s.bg, t().brand_muted);
        assert_eq!(s.fg, t().brand_muted_foreground);
        assert!(s.bold);
        assert_eq!(s.hover_bg, t().brand_muted);
    }

    #[test]
    fn unread_row_is_bold_with_sidebar_foreground() {
        let s = row_style(&t(), false, true, false);
        assert_eq!(s.bg, TRANSPARENT);
        assert_eq!(s.fg, t().sidebar_foreground);
        assert!(s.bold);
        assert_eq!(s.hover_bg, t().sidebar_accent);
    }

    #[test]
    fn plain_row_is_muted_and_hovers_to_accent() {
        let s = row_style(&t(), false, false, false);
        assert_eq!(s.fg, t().muted_foreground);
        assert!(!s.bold);
        assert_eq!(s.hover_fg, t().sidebar_accent_foreground);
    }

    #[test]
    fn dimmed_only_changes_plain_rows() {
        let plain = row_style(&t(), false, false, true);
        assert!(plain.fg.a < t().muted_foreground.a);
        assert_eq!(
            row_style(&t(), true, false, true),
            row_style(&t(), true, false, false)
        );
        assert_eq!(
            row_style(&t(), false, true, true),
            row_style(&t(), false, true, false)
        );
    }

    #[test]
    fn group_dm_is_detected_by_prefix() {
        assert!(is_group_dm("dm:group:abc"));
        assert!(!is_group_dm("dm:u1:u2"));
        assert!(!is_group_dm("edm:u1:u2"));
    }

    #[test]
    fn dm_name_sorts_and_limits_to_three() {
        let names = ["Dave", "Alice", "Carol", "Bob", "Eve"]
            .into_iter()
            .map(String::from)
            .collect();
        let label = dm_display_name(names, 3, "無題", |n| format!("+{n}"));
        assert_eq!(label, "Alice, Bob, Carol +2");
    }

    #[test]
    fn dm_name_falls_back_to_untitled() {
        assert_eq!(
            dm_display_name(vec![], 3, "無題", |n| format!("+{n}")),
            "無題"
        );
        assert_eq!(
            dm_display_name(vec![String::new()], 3, "無題", |n| format!("+{n}")),
            "無題"
        );
    }

    #[test]
    fn dm_name_sorts_case_insensitively() {
        let names = ["bob", "Alice", "carol", "Dave"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            dm_display_name(names, 10, "無題", |n| format!("+{n}")),
            "Alice, bob, carol, Dave"
        );
    }

    #[test]
    fn one_to_one_ids_are_parsed_from_the_name() {
        assert_eq!(parse_one_to_one_dm_ids("dm:u1:u2"), Some(("u1", "u2")));
        assert_eq!(parse_one_to_one_dm_ids("edm:u1:u2"), Some(("u1", "u2")));
        assert_eq!(parse_one_to_one_dm_ids("dm:group:abc"), None);
        assert_eq!(parse_one_to_one_dm_ids("dm:u1"), None);
        assert_eq!(parse_one_to_one_dm_ids("dm:u1:u2:u3"), None);
        assert_eq!(parse_one_to_one_dm_ids("general"), None);
    }

    #[test]
    fn one_to_one_uses_name_ids_even_without_member_rows() {
        assert_eq!(dm_other_ids("dm:me:bob", &[], "me"), ["bob"]);
        // 自分との DM は自分だけなので空になる。
        assert!(dm_other_ids("dm:me:me", &[], "me").is_empty());
        // グループはメンバー行を使う。
        let members = vec!["me".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(dm_other_ids("dm:group:x", &members, "me"), ["a", "b"]);
    }

    #[test]
    fn initials_follow_frontend_rules() {
        assert_eq!(initials("Alice Smith Jr"), "AS");
        assert_eq!(initials("alice"), "A");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("田中 太郎"), "田太");
    }
}
