//! `button.tsx` (cva) の移植。

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, SharedString, StyleRefinement, Window,
    div, prelude::*, px,
};


use super::color::mix_oklch;
use super::{Palette, TRANSPARENT, TextSpec, alpha, icon_colored, ring};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::{Tokens, radius};
use gpui::Rgba;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    Outline,
    Secondary,
    Ghost,
    Destructive,
    Link,
}

impl ButtonVariant {
    pub const ALL: [Self; 6] = [
        Self::Default,
        Self::Outline,
        Self::Secondary,
        Self::Ghost,
        Self::Destructive,
        Self::Link,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Outline => "outline",
            Self::Secondary => "secondary",
            Self::Ghost => "ghost",
            Self::Destructive => "destructive",
            Self::Link => "link",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    Default,
    Xs,
    Sm,
    Lg,
    Icon,
    IconXs,
    IconSm,
    IconLg,
}

impl ButtonSize {
    pub const ALL: [Self; 8] = [
        Self::Xs,
        Self::Sm,
        Self::Default,
        Self::Lg,
        Self::Icon,
        Self::IconXs,
        Self::IconSm,
        Self::IconLg,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Xs => "xs",
            Self::Sm => "sm",
            Self::Lg => "lg",
            Self::Icon => "icon",
            Self::IconXs => "icon-xs",
            Self::IconSm => "icon-sm",
            Self::IconLg => "icon-lg",
        }
    }
}

/// サイズごとの寸法 (px)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeSpec {
    pub height: f32,
    /// アイコンだけのサイズは正方形で、幅を持つ。
    pub width: Option<f32>,
    pub gap: f32,
    pub px: f32,
    /// `has-data-[icon=inline-start]:pl-*`
    pub px_icon_start: f32,
    /// `has-data-[icon=inline-end]:pr-*`
    pub px_icon_end: f32,
    pub radius: f32,
    pub text: TextSpec,
    pub icon: f32,
}

/// `rounded-[min(var(--radius-md),Npx)]`
fn min_md(cap: f32) -> f32 {
    f32::from(radius::MD).min(cap)
}

pub fn size_spec(size: ButtonSize) -> SizeSpec {
    let lg_radius = f32::from(radius::LG);
    let base = SizeSpec {
        height: 32.0,
        width: None,
        gap: 6.0,
        px: 10.0,
        px_icon_start: 8.0,
        px_icon_end: 8.0,
        radius: lg_radius,
        text: TextSpec::SM,
        icon: 16.0,
    };
    match size {
        ButtonSize::Default => base,
        ButtonSize::Xs => SizeSpec {
            height: 24.0,
            gap: 4.0,
            px: 8.0,
            px_icon_start: 6.0,
            px_icon_end: 6.0,
            radius: min_md(10.0),
            text: TextSpec::XS,
            icon: 12.0,
            ..base
        },
        ButtonSize::Sm => SizeSpec {
            height: 28.0,
            gap: 4.0,
            px: 10.0,
            px_icon_start: 6.0,
            px_icon_end: 6.0,
            radius: min_md(12.0),
            // `text-[0.8rem]`。行の高さは指定が無く、継承の 1.5 倍。
            text: TextSpec {
                size: 12.8,
                line: 19.2,
            },
            icon: 14.0,
            ..base
        },
        ButtonSize::Lg => SizeSpec {
            height: 36.0,
            ..base
        },
        ButtonSize::Icon => SizeSpec {
            width: Some(32.0),
            ..base
        },
        ButtonSize::IconXs => SizeSpec {
            height: 24.0,
            width: Some(24.0),
            radius: min_md(10.0),
            icon: 12.0,
            ..base
        },
        ButtonSize::IconSm => SizeSpec {
            height: 28.0,
            width: Some(28.0),
            radius: min_md(12.0),
            ..base
        },
        ButtonSize::IconLg => SizeSpec {
            height: 36.0,
            width: Some(36.0),
            ..base
        },
    }
}

/// 見た目を決める状態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub hovered: bool,
    pub focused: bool,
    pub expanded: bool,
    pub invalid: bool,
    pub disabled: bool,
}

/// 状態を反映した色など。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub bg: Rgba,
    pub fg: Rgba,
    pub border: Rgba,
    /// `focus-visible:ring-3` などの輪。(色、太さ)
    pub ring: Option<(Rgba, f32)>,
    pub opacity: f32,
    pub underline: bool,
}

pub fn resolve(variant: ButtonVariant, p: &Palette, s: ButtonState) -> Resolved {
    let t: &Tokens = &p.t;
    let dark = p.is_dark();
    let hovered = s.hovered && !s.disabled;
    let expanded = s.expanded;
    let (mut bg, mut fg, mut border) = match variant {
        ButtonVariant::Default => (t.primary, t.primary_foreground, TRANSPARENT),
        ButtonVariant::Outline => (
            p.pick(t.background, alpha(t.input, 0.3)),
            t.foreground,
            p.pick(t.border, t.input),
        ),
        ButtonVariant::Secondary => (t.secondary, t.secondary_foreground, TRANSPARENT),
        ButtonVariant::Ghost => (TRANSPARENT, t.foreground, TRANSPARENT),
        ButtonVariant::Destructive => (
            alpha(t.destructive, p.pick(0.1, 0.2)),
            t.destructive,
            TRANSPARENT,
        ),
        ButtonVariant::Link => (TRANSPARENT, t.link, TRANSPARENT),
    };
    // 文字色の既定は、outline と ghost だけ「foreground」で、hover でも変わらない。
    if variant == ButtonVariant::Outline || variant == ButtonVariant::Ghost {
        fg = t.foreground;
    }
    let mut underline = false;
    if hovered {
        match variant {
            ButtonVariant::Default => bg = t.primary_hover,
            ButtonVariant::Outline => bg = p.pick(t.muted, alpha(t.input, 0.5)),
            ButtonVariant::Secondary => bg = mix_oklch(t.secondary, t.foreground, 0.05),
            ButtonVariant::Ghost => bg = p.pick(t.muted, alpha(t.muted, 0.5)),
            ButtonVariant::Destructive => bg = alpha(t.destructive, p.pick(0.2, 0.3)),
            ButtonVariant::Link => underline = true,
        }
    }
    if expanded {
        match variant {
            ButtonVariant::Outline | ButtonVariant::Ghost => bg = t.muted,
            ButtonVariant::Secondary => bg = t.secondary,
            _ => {}
        }
    }
    let mut ring_style = None;
    if s.focused {
        if variant == ButtonVariant::Destructive {
            border = alpha(t.destructive, 0.4);
            ring_style = Some((alpha(t.destructive, p.pick(0.2, 0.4)), 3.0));
        } else {
            border = t.ring;
            ring_style = Some((alpha(t.ring, 0.5), 3.0));
        }
    }
    if s.invalid {
        border = p.pick(t.destructive, alpha(t.destructive, 0.5));
        ring_style = Some((alpha(t.destructive, p.pick(0.2, 0.4)), 3.0));
    }
    let mut opacity = 1.0;
    if s.disabled {
        if variant == ButtonVariant::Default {
            bg = t.muted;
            fg = t.muted_foreground;
        } else {
            opacity = 0.5;
        }
        underline = false;
    }
    let _ = dark;
    Resolved {
        bg,
        fg,
        border,
        ring: ring_style,
        opacity,
        underline,
    }
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    p: Palette,
    variant: ButtonVariant,
    size: ButtonSize,
    label: Option<SharedString>,
    icon_start: Option<Icon>,
    icon_end: Option<Icon>,
    state: ButtonState,
    radius: Option<f32>,
    size_override: Option<f32>,
    on_click: Option<ClickHandler>,
    focus_handle: Option<FocusHandle>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, p: &Palette) -> Self {
        Self {
            id: id.into(),
            p: *p,
            variant: ButtonVariant::Default,
            size: ButtonSize::Default,
            label: None,
            icon_start: None,
            icon_end: None,
            state: ButtonState::default(),
            radius: None,
            size_override: None,
            on_click: None,
            focus_handle: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// アイコンだけのボタン。サイズも `icon` 系にそろえること。
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon_start = Some(icon);
        self
    }

    pub fn icon_start(mut self, icon: Icon) -> Self {
        self.icon_start = Some(icon);
        self
    }

    pub fn icon_end(mut self, icon: Icon) -> Self {
        self.icon_end = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    /// hover の見た目を常に出す。確認画面用。
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.state.hovered = hovered;
        self
    }

    /// `focus-visible` の見た目を常に出す。
    pub fn focused(mut self, focused: bool) -> Self {
        self.state.focused = focused;
        self
    }

    /// `aria-expanded` の見た目。
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.state.expanded = expanded;
        self
    }

    /// `aria-invalid` の見た目。
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.state.invalid = invalid;
        self
    }

    /// 角丸を上書きする (ButtonGroup の項目が使う)。
    pub fn rounded_px(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// 正方形の一辺を上書きする (ButtonGroup の項目が 40px にする)。
    pub fn square_px(mut self, size: f32) -> Self {
        self.size_override = Some(size);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// 実際のフォーカスで輪を出すために、フォーカスハンドルを結ぶ。
    pub fn focus_handle(mut self, handle: &FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut sp = size_spec(self.size);
        if let Some(r) = self.radius {
            sp.radius = r;
        }
        if let Some(side) = self.size_override {
            sp.height = side;
            sp.width = Some(side);
        }
        let normal = resolve(self.variant, &self.p, self.state);
        let hover = resolve(
            self.variant,
            &self.p,
            ButtonState {
                hovered: true,
                ..self.state
            },
        );
        let live_hover = !self.state.disabled && !self.state.hovered;
        let focus_state = resolve(
            self.variant,
            &self.p,
            ButtonState {
                focused: true,
                ..self.state
            },
        );
        let group: SharedString = format!("button-{}", self.id).into();

        let has_start = self.icon_start.is_some() && self.label.is_some();
        let has_end = self.icon_end.is_some() && self.label.is_some();
        let pl = if has_start { sp.px_icon_start } else { sp.px };
        let pr = if has_end { sp.px_icon_end } else { sp.px };

        // `bg-clip-padding`: 背景は枠の内側だけに塗るので、背景は 1px 内側の層に描く。
        let layer = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .rounded(px((sp.radius - 1.0).max(0.0)))
            .bg(normal.bg)
            .when(live_hover, |d| {
                d.group_hover(group.clone(), |s| s.bg(hover.bg))
            });

        let mut root = div()
            .id(self.id)
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(sp.gap))
            .h(px(sp.height))
            .when_some(sp.width, |d, w| d.w(px(w)))
            .when(sp.width.is_none(), |d| d.pl(px(pl)).pr(px(pr)))
            .rounded(px(sp.radius))
            .border_1()
            .border_color(normal.border)
            .text_size(sp.text.size())
            .line_height(sp.text.line())
            .font(fonts::sans(500.0))
            .text_color(normal.fg)
            .whitespace_nowrap()
            .opacity(normal.opacity)
            .when(normal.underline, |d| d.underline())
            .when_some(normal.ring, |d, (c, w)| d.shadow(vec![ring(c, w)]))
            .when(live_hover, |d| {
                d.hover(|s| {
                    let s = s.text_color(hover.fg);
                    if hover.underline { s.underline() } else { s }
                })
            })
            .when(!self.state.disabled, |d| {
                d.active(|s| s.top(px(1.0)))
            })
            .when_some(self.focus_handle.as_ref(), |d, handle| {
                d.track_focus(handle).focus(move |s| {
                    let s = s.border_color(focus_state.border);
                    match focus_state.ring {
                        Some((c, w)) => s.shadow(vec![ring(c, w)]),
                        None => s,
                    }
                })
            })
            .child(layer)
            .when_some(self.icon_start, |d, ic| d.child(icon_colored(ic, sp.icon, normal.fg)))
            .when_some(self.label, |d, label| d.child(label))
            .children(self.children)
            .when_some(self.icon_end, |d, ic| d.child(icon_colored(ic, sp.icon, normal.fg)));

        if let Some(on_click) = self.on_click
            && !self.state.disabled
        {
            root = root.on_click(move |e, w, cx| on_click(e, w, cx));
        }
        root.style().refine(&self.style);
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{ThemeMode, to_u8};

    fn light() -> Palette {
        Palette::new(ThemeMode::Light)
    }

    #[test]
    fn heights_follow_tailwind_spacing() {
        let heights: Vec<_> = ButtonSize::ALL
            .iter()
            .map(|s| (s.name(), size_spec(*s).height))
            .collect();
        assert_eq!(
            heights,
            [
                ("xs", 24.0),
                ("sm", 28.0),
                ("default", 32.0),
                ("lg", 36.0),
                ("icon", 32.0),
                ("icon-xs", 24.0),
                ("icon-sm", 28.0),
                ("icon-lg", 36.0),
            ]
        );
    }

    #[test]
    fn icon_sizes_are_square() {
        for s in [
            ButtonSize::Icon,
            ButtonSize::IconXs,
            ButtonSize::IconSm,
            ButtonSize::IconLg,
        ] {
            let sp = size_spec(s);
            assert_eq!(sp.width, Some(sp.height), "{}", s.name());
        }
        assert_eq!(size_spec(ButtonSize::Default).width, None);
    }

    #[test]
    fn radius_is_capped_by_min_md() {
        assert_eq!(size_spec(ButtonSize::Default).radius, 10.0);
        assert_eq!(size_spec(ButtonSize::Xs).radius, 8.0);
        assert_eq!(size_spec(ButtonSize::Sm).radius, 8.0);
        assert_eq!(size_spec(ButtonSize::IconXs).radius, 8.0);
        assert_eq!(size_spec(ButtonSize::IconSm).radius, 8.0);
        assert_eq!(size_spec(ButtonSize::IconLg).radius, 10.0);
    }

    #[test]
    fn icon_and_padding_per_size() {
        assert_eq!(size_spec(ButtonSize::Xs).icon, 12.0);
        assert_eq!(size_spec(ButtonSize::Sm).icon, 14.0);
        assert_eq!(size_spec(ButtonSize::Default).icon, 16.0);
        assert_eq!(size_spec(ButtonSize::IconSm).icon, 16.0);
        assert_eq!(size_spec(ButtonSize::Default).px_icon_start, 8.0);
        assert_eq!(size_spec(ButtonSize::Xs).px_icon_start, 6.0);
    }

    #[test]
    fn default_variant_hover_and_disabled() {
        let p = light();
        let normal = resolve(ButtonVariant::Default, &p, ButtonState::default());
        assert_eq!(normal.bg, p.t.primary);
        let hover = resolve(
            ButtonVariant::Default,
            &p,
            ButtonState {
                hovered: true,
                ..Default::default()
            },
        );
        assert_eq!(hover.bg, p.t.primary_hover);
        // 無効のときは opacity を下げず、muted の配色にする。
        let disabled = resolve(
            ButtonVariant::Default,
            &p,
            ButtonState {
                disabled: true,
                hovered: true,
                ..Default::default()
            },
        );
        assert_eq!(disabled.bg, p.t.muted);
        assert_eq!(disabled.fg, p.t.muted_foreground);
        assert_eq!(disabled.opacity, 1.0);
    }

    #[test]
    fn other_variants_fade_when_disabled() {
        let p = light();
        let d = resolve(
            ButtonVariant::Outline,
            &p,
            ButtonState {
                disabled: true,
                ..Default::default()
            },
        );
        assert_eq!(d.opacity, 0.5);
    }

    #[test]
    fn outline_changes_with_mode() {
        let l = resolve(ButtonVariant::Outline, &light(), ButtonState::default());
        let d = resolve(
            ButtonVariant::Outline,
            &Palette::new(ThemeMode::Dark),
            ButtonState::default(),
        );
        assert_eq!(l.border, light().t.border);
        assert_eq!(l.bg, light().t.background);
        assert_eq!(d.border, Palette::new(ThemeMode::Dark).t.input);
        assert!((d.bg.a - 0.3).abs() < 1e-6);
    }

    #[test]
    fn secondary_hover_mixes_five_percent_foreground() {
        let p = light();
        let hover = resolve(
            ButtonVariant::Secondary,
            &p,
            ButtonState {
                hovered: true,
                ..Default::default()
            },
        );
        let base = to_u8(p.t.secondary);
        let mixed = to_u8(hover.bg);
        assert!(mixed[0] < base[0]);
    }

    #[test]
    fn destructive_focus_and_invalid_rings() {
        let p = light();
        let f = resolve(
            ButtonVariant::Destructive,
            &p,
            ButtonState {
                focused: true,
                ..Default::default()
            },
        );
        assert!((f.border.a - 0.4).abs() < 1e-6);
        assert_eq!(f.ring.map(|r| r.1), Some(3.0));
        let i = resolve(
            ButtonVariant::Default,
            &p,
            ButtonState {
                invalid: true,
                ..Default::default()
            },
        );
        assert_eq!(i.border, p.t.destructive);
    }

    #[test]
    fn link_underlines_only_on_hover() {
        let p = light();
        assert!(!resolve(ButtonVariant::Link, &p, ButtonState::default()).underline);
        let h = ButtonState {
            hovered: true,
            ..Default::default()
        };
        assert!(resolve(ButtonVariant::Link, &p, h).underline);
        assert_eq!(resolve(ButtonVariant::Link, &p, h).fg, p.t.link);
    }
}
