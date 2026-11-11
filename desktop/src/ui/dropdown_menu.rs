//! `dropdown-menu.tsx`。開いたメニューの見た目。位置決めは呼び出し側 (`deferred` + `anchored`、
//! または `relative` な親の中の `absolute`) で行う。

use gpui::{
    AnyElement, App, ClickEvent, ElementId, SharedString, StyleRefinement, Window, div, prelude::*,
    px,
};

use super::{ClickHandler, Palette, TextSpec, alpha, icon_colored, ring, shadow_md};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;

/// `min-w-44`
pub const MIN_WIDTH: f32 = 176.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemVariant {
    Default,
    Destructive,
}

/// 項目の色。(文字、背景、アイコン)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemColors {
    pub fg: gpui::Rgba,
    pub bg: gpui::Rgba,
    pub icon: gpui::Rgba,
}

/// `focus:bg-accent focus:text-accent-foreground` と destructive 版。
pub fn item_colors(p: &Palette, variant: ItemVariant, highlighted: bool) -> ItemColors {
    let t = p.t;
    match (variant, highlighted) {
        (ItemVariant::Default, false) => ItemColors {
            fg: t.popover_foreground,
            bg: super::TRANSPARENT,
            icon: t.popover_foreground,
        },
        (ItemVariant::Default, true) => ItemColors {
            fg: t.accent_foreground,
            bg: t.accent,
            icon: t.accent_foreground,
        },
        (ItemVariant::Destructive, false) => ItemColors {
            fg: t.destructive,
            bg: super::TRANSPARENT,
            icon: t.destructive,
        },
        (ItemVariant::Destructive, true) => ItemColors {
            fg: t.destructive,
            bg: alpha(t.destructive, p.pick(0.1, 0.2)),
            icon: t.destructive,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    None,
    /// チェックボックス項目 (右端に Check)。
    Checkbox(bool),
    /// ラジオ項目 (右端に Check)。
    Radio(bool),
    /// サブメニューの開閉 (右端に ChevronRight)。
    Submenu,
}

#[derive(IntoElement)]
pub struct MenuItem {
    id: ElementId,
    p: Palette,
    label: SharedString,
    icon: Option<Icon>,
    shortcut: Option<SharedString>,
    variant: ItemVariant,
    indicator: Indicator,
    highlighted: bool,
    disabled: bool,
    inset: bool,
    on_click: Option<ClickHandler>,
}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, p: &Palette, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            p: *p,
            label: label.into(),
            icon: None,
            shortcut: None,
            variant: ItemVariant::Default,
            indicator: Indicator::None,
            highlighted: false,
            disabled: false,
            inset: false,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn destructive(mut self) -> Self {
        self.variant = ItemVariant::Destructive;
        self
    }

    pub fn indicator(mut self, indicator: Indicator) -> Self {
        self.indicator = indicator;
        self
    }

    /// `focus:` の見た目を常に出す。
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `data-inset:pl-7`
    pub fn inset(mut self, inset: bool) -> Self {
        self.inset = inset;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for MenuItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let sm = TextSpec::SM;
        let c = item_colors(&self.p, self.variant, self.highlighted);
        let hover = item_colors(&self.p, self.variant, true);
        let has_indicator = matches!(self.indicator, Indicator::Checkbox(_) | Indicator::Radio(_));
        // 通常は `px-1.5`、チェック・ラジオは `pr-8 pl-1.5`、inset は `pl-7`。
        let pl = if self.inset { 28.0 } else { 6.0 };
        let pr = if has_indicator { 32.0 } else { 6.0 };
        let checked = matches!(
            self.indicator,
            Indicator::Checkbox(true) | Indicator::Radio(true)
        );
        let live = !self.disabled && !self.highlighted;
        // `relative flex items-center gap-1.5 rounded-md px-1.5 py-1 text-sm whitespace-nowrap`
        let mut item = div()
            .id(self.id)
            .relative()
            .flex()
            .items_center()
            .gap(px(6.0))
            .pl(px(pl))
            .pr(px(pr))
            .py(px(4.0))
            .rounded(radius::MD)
            .text_size(sm.size())
            .line_height(sm.line())
            .font(fonts::sans(400.0))
            .whitespace_nowrap()
            .text_color(c.fg)
            .bg(c.bg)
            .opacity(if self.disabled { 0.5 } else { 1.0 })
            .when(live, |d| {
                d.hover(move |s| s.bg(hover.bg).text_color(hover.fg))
            })
            .when_some(self.icon, |d, ic| d.child(icon_colored(ic, 16.0, c.icon)))
            .child(self.label)
            .when_some(self.shortcut, |d, text| {
                // `ml-auto text-xs tracking-widest text-muted-foreground`
                d.child(
                    div()
                        .ml_auto()
                        .text_size(TextSpec::XS.size())
                        .line_height(TextSpec::XS.line())
                        .text_color(if self.highlighted {
                            t.accent_foreground
                        } else {
                            t.muted_foreground
                        })
                        .child(text),
                )
            })
            .when(checked, |d| {
                // `absolute right-2` のチェック。
                d.child(
                    div()
                        .absolute()
                        .right(px(8.0))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(icon_colored(Icon::Check, 16.0, c.icon)),
                )
            })
            .when(self.indicator == Indicator::Submenu, |d| {
                d.child(
                    div()
                        .ml_auto()
                        .child(icon_colored(Icon::ChevronRight, 16.0, c.icon)),
                )
            });
        if let Some(h) = self.on_click
            && !self.disabled
        {
            item = item.on_click(move |e, w, cx| h(e, w, cx));
        }
        item
    }
}

/// `DropdownMenuLabel`: `px-1.5 py-1 text-xs font-medium text-muted-foreground`
pub fn menu_label(p: &Palette, text: impl Into<SharedString>, inset: bool) -> impl IntoElement {
    div()
        .pl(px(if inset { 28.0 } else { 6.0 }))
        .pr(px(6.0))
        .py(px(4.0))
        .text_size(TextSpec::XS.size())
        .line_height(TextSpec::XS.line())
        .font(fonts::sans(500.0))
        .text_color(p.t.muted_foreground)
        .child(text.into())
}

/// `DropdownMenuSeparator`: `-mx-1 my-1 h-px bg-border`
pub fn menu_separator(p: &Palette) -> impl IntoElement {
    div().mx(px(-4.0)).my(px(4.0)).h(px(1.0)).bg(p.t.border)
}

/// メニューの箱。`min-w-44 rounded-lg bg-popover p-1 shadow-md ring-1 ring-foreground/10`
#[derive(IntoElement)]
pub struct DropdownMenuContent {
    p: Palette,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl DropdownMenuContent {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.children.push(item.into_any_element());
        self
    }

    pub fn label(mut self, text: impl Into<SharedString>) -> Self {
        let el = menu_label(&self.p, text, false).into_any_element();
        self.children.push(el);
        self
    }

    pub fn separator(mut self) -> Self {
        let el = menu_separator(&self.p).into_any_element();
        self.children.push(el);
        self
    }
}

impl ParentElement for DropdownMenuContent {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for DropdownMenuContent {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// ポップアップの共通の枠 (メニュー、セレクトの一覧)。
pub fn popup_frame(p: &Palette) -> gpui::Div {
    let t = p.t;
    let mut shadows = shadow_md();
    shadows.push(ring(alpha(t.foreground, 0.1), 1.0));
    div()
        .flex()
        .flex_col()
        .min_w(px(MIN_WIDTH))
        .p(px(4.0))
        .rounded(radius::LG)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .shadow(shadows)
}

impl RenderOnce for DropdownMenuContent {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut menu = popup_frame(&self.p).children(self.children);
        menu.style().refine(&self.style);
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::ThemeMode;

    #[test]
    fn highlighted_default_uses_accent() {
        let p = Palette::new(ThemeMode::Light);
        let c = item_colors(&p, ItemVariant::Default, true);
        assert_eq!(c.bg, p.t.accent);
        assert_eq!(c.fg, p.t.accent_foreground);
        assert_eq!(c.icon, p.t.accent_foreground);
    }

    #[test]
    fn destructive_highlight_depends_on_mode() {
        let l = item_colors(
            &Palette::new(ThemeMode::Light),
            ItemVariant::Destructive,
            true,
        );
        let d = item_colors(
            &Palette::new(ThemeMode::Dark),
            ItemVariant::Destructive,
            true,
        );
        assert!((l.bg.a - 0.1).abs() < 1e-6);
        assert!((d.bg.a - 0.2).abs() < 1e-6);
        assert_eq!(l.fg, Palette::new(ThemeMode::Light).t.destructive);
    }

    #[test]
    fn idle_item_is_transparent() {
        let p = Palette::new(ThemeMode::Light);
        assert_eq!(item_colors(&p, ItemVariant::Default, false).bg.a, 0.0);
    }
}
