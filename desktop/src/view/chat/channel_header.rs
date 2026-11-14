//! チャンネルヘッダー (frontend の `chat/ChannelHeader.tsx` と `layout/PaneHeader.tsx` に対応)。
//!
//! 写していないもの: 通話ボタン (通話の機能がない)、メンバーの開閉 (MembersPanel がない)、
//! チャンネル設定・DM のメニュー・アーカイブ操作 (ダイアログがまだない)。
//! 背景は `bg-background/80` を `backdrop-blur` なしで描く (GPUI にぼかしがない)。

use gpui::{Context, Div, ParentElement, Styled, div, prelude::*, px};

use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::pane_header::PaneHeader;
use crate::ui::{TextSpec, alpha, h_flex, icon_colored, palette};
use crate::view::WorkspaceView;

/// ヘッダーに出すチャンネルの情報。
pub struct ChannelHeaderInfo {
    pub title: String,
    pub description: Option<String>,
    /// グループ DM の参加者数 (サブタイトルの `dms.memberCount` に使う)。
    pub member_count: usize,
    pub is_dm: bool,
    pub is_group_dm: bool,
    pub is_private: bool,
    pub is_nsfw: bool,
    pub is_archived: bool,
    pub failed_draft_count: usize,
    pub can_join: bool,
    pub can_leave: bool,
    pub is_member: bool,
    pub channel_id: String,
}

/// 左端の枠に入れるアイコン (`ChannelHeader.tsx`)。
pub fn header_icon(is_dm: bool, is_group_dm: bool, is_private: bool) -> Icon {
    if is_dm {
        if is_group_dm {
            Icon::Users
        } else {
            Icon::MessagesSquare
        }
    } else if is_private {
        Icon::Lock
    } else {
        Icon::Hash
    }
}

/// サブタイトル (`ChannelScreen.tsx`)。DM は 1 対 1 なら `dms.direct`、グループなら人数。
/// 通常のチャンネルは説明で、無ければ `channels.noDescription`。
pub fn header_subtitle(info: &ChannelHeaderInfo, i18n: &crate::i18n::I18n) -> String {
    if info.is_dm {
        if info.is_group_dm {
            i18n.t_count("dms.memberCount", info.member_count as i64, &[])
        } else {
            i18n.t("dms.direct")
        }
    } else {
        info.description
            .clone()
            .unwrap_or_else(|| i18n.t("channels.noDescription"))
    }
}

impl WorkspaceView {
    pub(in crate::view) fn render_channel_header(
        &self,
        info: ChannelHeaderInfo,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        let xs = TextSpec::XS;
        let subtitle = header_subtitle(&info, &i18n);
        let ch_id = info.channel_id.clone();
        let is_member = info.is_member;

        let header = PaneHeader::new()
            .bg(alpha(t.background, 0.8))
            .child(
                // `flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground`
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(radius::LG)
                    .bg(t.muted)
                    .child(icon_colored(
                        header_icon(info.is_dm, info.is_group_dm, info.is_private),
                        16.0,
                        t.muted_foreground,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(sm.size())
                                    .line_height(sm.line())
                                    .font(fonts::sans(600.0))
                                    .child(info.title.clone()),
                            )
                            .when(info.is_nsfw, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(xs.size())
                                        .line_height(xs.line())
                                        .text_color(t.muted_foreground)
                                        .child("NSFW"),
                                )
                            })
                            .when(info.is_archived && !info.is_dm, |d| {
                                // `shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground`
                                d.child(
                                    div()
                                        .flex_none()
                                        .px(px(6.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .bg(t.muted)
                                        .text_size(px(10.4))
                                        .line_height(px(14.0))
                                        .text_color(t.muted_foreground)
                                        .child(i18n.t("moderation.archived")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .text_color(t.muted_foreground)
                            .child(subtitle),
                    ),
            )
            .child(
                // `flex shrink-0 items-center gap-1`
                h_flex()
                    .flex_none()
                    .gap(px(4.0))
                    .when(info.failed_draft_count > 0, |d| {
                        d.child(
                            Button::new("btn-restore-draft")
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .label(format!("下書きを復元 ({})", info.failed_draft_count))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.restore_failed_draft();
                                        cx.notify();
                                    });
                                })),
                        )
                    })
                    .when(info.can_join || info.can_leave, |d| {
                        d.child(
                            Button::new("btn-join-or-leave")
                                .variant(if is_member {
                                    ButtonVariant::Outline
                                } else {
                                    ButtonVariant::Default
                                })
                                .size(ButtonSize::Sm)
                                .label(if is_member {
                                    i18n.t("members.leave")
                                } else {
                                    i18n.t("channels.join")
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let ch = ch_id.clone();
                                    this.state.update(cx, |s, cx| {
                                        s.join_or_leave(ch, !is_member);
                                        cx.notify();
                                    });
                                })),
                        )
                    })
                    .child(
                        Button::new("btn-open-search")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon_start(Icon::Search)
                            .label(i18n.t("search.action"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_message_search(window, cx);
                            })),
                    ),
            );
        div().child(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(is_dm: bool, is_group_dm: bool, description: Option<&str>) -> ChannelHeaderInfo {
        ChannelHeaderInfo {
            title: "t".into(),
            description: description.map(String::from),
            member_count: 4,
            is_dm,
            is_group_dm,
            is_private: false,
            is_nsfw: false,
            is_archived: false,
            failed_draft_count: 0,
            can_join: false,
            can_leave: false,
            is_member: true,
            channel_id: "c".into(),
        }
    }

    #[test]
    fn subtitle_follows_channel_kind() {
        use crate::i18n::{I18n, Locale};
        let ja = I18n::new(Locale::Ja);
        assert_eq!(
            header_subtitle(&info(true, false, None), &ja),
            ja.t("dms.direct")
        );
        assert_eq!(
            header_subtitle(&info(true, true, Some("x")), &ja),
            ja.t_count("dms.memberCount", 4, &[])
        );
        assert!(header_subtitle(&info(true, true, None), &ja).contains('4'));
        assert_eq!(
            header_subtitle(&info(false, false, Some("説明")), &ja),
            "説明"
        );
        assert_eq!(
            header_subtitle(&info(false, false, None), &ja),
            ja.t("channels.noDescription")
        );
    }

    #[test]
    fn icon_follows_channel_kind() {
        assert_eq!(header_icon(false, false, false), Icon::Hash);
        assert_eq!(header_icon(false, false, true), Icon::Lock);
        assert_eq!(header_icon(true, false, true), Icon::MessagesSquare);
        assert_eq!(header_icon(true, true, true), Icon::Users);
    }
}
