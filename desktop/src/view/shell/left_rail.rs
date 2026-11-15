//! 最左のレール (frontend の `layout/LeftRail.tsx` に対応)。
//!
//! 幅 64px (`md:w-16`)、ロゴ 36px、ナビのボタン 40px。
//! デスクトップにある行き先だけを出す。Home、通知、Bubble、Apps、設定の画面はまだなく、
//! 出すと押しても何も起きないので、レールには並べない。

use gpui::{
    AnyElement, Context, Div, ElementId, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use zoo2_client_core::config::AuthConfig;

use crate::assets::logo;
use crate::icons::Icon;
use crate::tokens::{ThemeMode, radius};
use crate::ui::avatar::{Avatar, AvatarSize};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::pane_header::PaneHeader;
use crate::ui::separator::Separator;
use crate::ui::{h_flex, icon_colored, palette, v_flex};
use crate::view::WorkspaceView;
use crate::view::shell::sidebar::initials;

/// レールの幅 (`md:w-16`)。
pub const RAIL_WIDTH: f32 = 64.0;
/// ナビのボタン (`size-10`)。
pub const NAV_BUTTON: f32 = 40.0;
/// ロゴ (`size-9`)。
pub const LOGO_SIZE: f32 = 36.0;

/// レールに出す行き先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailDestination {
    /// チャンネルと DM (`/`)。
    Channels,
}

impl RailDestination {
    pub fn icon(self) -> Icon {
        match self {
            Self::Channels => Icon::Hash,
        }
    }

    /// 翻訳のキー。
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Channels => "channels.title",
        }
    }
}

/// デスクトップが実装している行き先。
pub fn rail_destinations() -> &'static [RailDestination] {
    &[RailDestination::Channels]
}

/// ログアウトのボタンを出すか。OIDC のときだけ出す (dev は出さない)。
/// Web は #495 でログアウトとテーマの切り替えを設定画面へ移したが、デスクトップには設定画面が
/// まだないので、レールの下部に残す (設定画面を作るときに移す)。
pub fn show_logout(auth: &AuthConfig) -> bool {
    matches!(auth, AuthConfig::Oidc(_))
}

impl WorkspaceView {
    pub(in crate::view) fn render_left_rail(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let t = p.t;
        let state = self.state.read(cx);
        let logout = show_logout(&state.config.auth);
        let me_label = state
            .me
            .as_ref()
            .map(|m| {
                if m.display_name.is_empty() {
                    m.user_name.clone()
                } else {
                    m.display_name.clone()
                }
            })
            .unwrap_or_else(|| "?".to_string());
        let mode = state.theme_mode;

        let nav: Vec<AnyElement> = rail_destinations()
            .iter()
            .map(|dest| {
                // 今はチャンネル画面だけなので、常に現在地として強調する。
                let (bg, fg) = (t.brand_muted, t.brand_muted_foreground);
                // `relative flex size-10 items-center justify-center rounded-xl text-muted-foreground`
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "rail-{}",
                        dest.label_key()
                    ))))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(NAV_BUTTON))
                    .rounded(radius::XL)
                    .bg(bg)
                    .child(icon_colored(dest.icon(), 20.0, fg))
                    .into_any_element()
            })
            .collect();

        // `flex w-14 shrink-0 flex-col items-center gap-1 border-r bg-sidebar pb-3 md:w-16`
        v_flex()
            .flex_none()
            .w(px(RAIL_WIDTH))
            .h_full()
            .items_center()
            .gap(px(4.0))
            .border_r_1()
            .border_color(t.border)
            .bg(t.sidebar)
            .pb(px(12.0))
            .child(
                // `mb-1 w-full justify-center px-0`。Web ではこの下に `gap-1` も足されて 8px 空くが、
                // GPUI のこの列では最初の子と次の子の間に `gap` が入らないので、`mb` で 8px にする。
                PaneHeader::new()
                    .mb(px(8.0))
                    .w_full()
                    .justify_center()
                    .px_0()
                    .child(logo(px(LOGO_SIZE)).rounded(radius::XL)),
            )
            .children(nav)
            .child(
                // `mt-auto flex flex-col items-center gap-2`
                v_flex()
                    .mt_auto()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        h_flex().child(
                            Button::new("btn-toggle-theme")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::IconSm)
                                .icon_start(if mode == crate::theme::ThemeMode::Dark {
                                    Icon::Moon
                                } else {
                                    Icon::Sun
                                })
                                .focus_handle(&self.theme_toggle_focus)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.toggle_theme();
                                        cx.notify();
                                    });
                                })),
                        ),
                    )
                    .child(Separator::new().w(px(24.0)))
                    .child(
                        Avatar::new()
                            .size(AvatarSize::Sm)
                            .initials(initials(&me_label)),
                    )
                    .when(logout, |d| {
                        d.child(
                            h_flex().child(
                                Button::new("btn-logout")
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::IconSm)
                                    .icon_start(Icon::LogOut)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.logout();
                                            cx.notify();
                                        });
                                    })),
                            ),
                        )
                    }),
            )
    }
}

/// `theme.rs` の旧モードをトークンのモードへ。
pub fn token_mode(mode: crate::theme::ThemeMode) -> ThemeMode {
    match mode {
        crate::theme::ThemeMode::Dark => ThemeMode::Dark,
        crate::theme::ThemeMode::Light => ThemeMode::Light,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoo2_client_core::config::OidcConfig;

    #[test]
    fn only_implemented_destinations_are_listed() {
        assert_eq!(rail_destinations(), &[RailDestination::Channels]);
        assert_eq!(RailDestination::Channels.icon(), Icon::Hash);
    }

    #[test]
    fn logout_only_for_oidc() {
        assert!(!show_logout(&AuthConfig::Dev {
            user_id: "u".into()
        }));
        assert!(show_logout(&AuthConfig::Oidc(OidcConfig {
            issuer: "https://a".into(),
            client_id: "c".into(),
            resource: None,
            scope: "openid".into(),
        })));
    }

    #[test]
    fn token_mode_maps_both_ways() {
        assert_eq!(token_mode(crate::theme::ThemeMode::Dark), ThemeMode::Dark);
        assert_eq!(token_mode(crate::theme::ThemeMode::Light), ThemeMode::Light);
    }

    #[test]
    fn sizes_follow_the_web_rail() {
        assert_eq!(RAIL_WIDTH, 16.0 * 4.0);
        assert_eq!(NAV_BUTTON, 10.0 * 4.0);
        assert_eq!(LOGO_SIZE, 9.0 * 4.0);
    }
}
