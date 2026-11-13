//! サイドバー (frontend の `ChannelSidebar` / `DmSection` に対応)。

use gpui::{
    Context, Div, ElementId, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px,
};
use zoo2_client_core::{config::AuthConfig, electric::ShapeId};

use crate::theme;
use crate::view::{WorkspaceView, is_activate_key};

impl WorkspaceView {
    pub(crate) fn render_sidebar_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_summary = state
            .me
            .as_ref()
            .map(|m| format!("{} (@{})", m.display_name, m.user_name))
            .unwrap_or_default();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));
        let channels_synced = state.store.is_synced(&ShapeId::Channels);
        let sync_error = state.sync_error();
        let notice = state.notice.as_ref().map(|(m, _)| m.clone());
        let current_channel = state.current_channel.clone();
        let unread = state.unread.clone();

        let filtered = state.filtered_channels();
        let mut normal_rows = Vec::new();
        let mut dm_rows = Vec::new();

        let theme = state.theme();

        for ch in filtered {
            let ch_id = ch.id.clone();
            let ch_id_click = ch_id.clone();
            let is_dm = ch.is_dm;
            let is_open = current_channel.as_deref() == Some(ch_id.as_str());
            let is_unread = unread.contains(&ch_id);
            let is_member = state.store.is_member(&ch_id, &me_id);
            let mut label = state.store.channel_label(ch, &me_id);
            if ch.is_private && !ch.is_dm {
                label.push_str(" 🔒");
            }
            if ch.archive_at.is_some() {
                label.push_str(" (アーカイブ)");
            }

            let row_bg = if is_open {
                theme.bg_selected
            } else {
                theme.bg_secondary
            };
            let text_color = if is_open || is_unread {
                theme.text_primary
            } else if is_member {
                theme.text_muted
            } else {
                theme.text_dim
            };
            let weight = if is_open || is_unread {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };

            let row = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "sidebar-ch-{ch_id}"
                ))))
                .w_full()
                .px_2p5()
                .py_1p5()
                .rounded_md()
                .bg(row_bg)
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let id = ch_id_click.clone();
                        this.state.update(cx, |s, cx| {
                            s.open_channel(id);
                            cx.notify();
                        });
                    }),
                )
                .child(
                    div()
                        .w(px(10.0))
                        .text_xs()
                        .text_color(theme.accent)
                        .child(if is_unread { "●" } else { "" }),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_sm()
                        .font_weight(weight)
                        .text_color(text_color)
                        .child(label),
                );

            if is_dm {
                dm_rows.push(row);
            } else {
                normal_rows.push(row);
            }
        }

        div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .h_full()
            .bg(theme.bg_secondary)
            .border_r_1()
            .border_color(theme.border)
            // サイドバーヘッダー
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child("zoo2"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .when(!state.is_current_user_guest(), |d| {
                                        d.child(
                                            div()
                                                .id("btn-open-create-channel")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(theme.bg_surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme.text_primary)
                                                .child("+ 作成")
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(
                                                        |this, _: &MouseUpEvent, window, cx| {
                                                            this.open_create_channel_modal(
                                                                window, cx,
                                                            );
                                                        },
                                                    ),
                                                ),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("btn-toggle-theme")
                                            .track_focus(&self.theme_toggle_focus)
                                            .focus(|s| {
                                                s.border_color(theme.border_accent)
                                                    .bg(theme.bg_hover)
                                            })
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_sm()
                                            .bg(theme.bg_surface)
                                            .border_1()
                                            .border_color(theme.border)
                                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                            .text_xs()
                                            .child(if state.theme_mode == theme::ThemeMode::Dark {
                                                "🌙"
                                            } else {
                                                "☀️"
                                            })
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                    this.state.update(cx, |s, cx| {
                                                        s.toggle_theme();
                                                        cx.notify();
                                                    });
                                                }),
                                            )
                                            .on_key_down(cx.listener(
                                                |this, ev: &KeyDownEvent, _, cx| {
                                                    if is_activate_key(&ev.keystroke.key) {
                                                        cx.stop_propagation();
                                                        this.state.update(cx, |s, cx| {
                                                            s.toggle_theme();
                                                            cx.notify();
                                                        });
                                                    }
                                                },
                                            )),
                                    )
                                    .when(is_oidc, |d| {
                                        d.child(
                                            div()
                                                .id("btn-logout")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(theme.bg_surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme.text_muted)
                                                .child("ログアウト")
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                        this.state.update(cx, |s, cx| {
                                                            s.logout();
                                                            cx.notify();
                                                        });
                                                    }),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(me_summary),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(div().flex_1().child(self.search_input.clone()))
                            .child(
                                div()
                                    .id("btn-open-quick-switcher")
                                    .px_2()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("⌘K")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                                            this.open_quick_switcher(window, cx);
                                        }),
                                    ),
                            ),
                    ),
            )
            // チャンネル・DM 一覧
            .child(
                div()
                    .id("sidebar-scroll-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_muted)
                            .child(if channels_synced {
                                "チャンネル"
                            } else if sync_error.is_some() {
                                "チャンネル (同期停止)"
                            } else {
                                "チャンネル (同期中…)"
                            }),
                    )
                    .when_some(sync_error, |d, err| {
                        d.child(
                            div()
                                .mx_2()
                                .my_1()
                                .p_2()
                                .rounded_md()
                                .bg(theme.bg_surface)
                                .border_1()
                                .border_color(theme.danger)
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(div().text_xs().text_color(theme.warning).child(err))
                                .child(
                                    div()
                                        .id("btn-sidebar-retry-sync")
                                        .px_2()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(theme.accent_button)
                                        .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                        .text_center()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_on_accent)
                                        .child("再同期する")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.retry_sync();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                ),
                        )
                    })
                    .when(current_channel.is_none(), |d| {
                        d.when_some(notice, |d, msg| {
                            d.child(
                                div()
                                    .mx_2()
                                    .my_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.warning)
                                    .text_xs()
                                    .text_color(theme.warning)
                                    .child(msg),
                            )
                        })
                    })
                    .children(normal_rows)
                    .when(!dm_rows.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_2()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_muted)
                                .child("ダイレクトメッセージ"),
                        )
                        .children(dm_rows)
                    }),
            )
    }
}
