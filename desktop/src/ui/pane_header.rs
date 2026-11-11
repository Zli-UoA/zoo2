//! `layout/PaneHeader.tsx`。高さ 61px (`--spacing(15)` + 1px の下線)。

use gpui::{AnyElement, App, SharedString, StyleRefinement, Window, div, prelude::*, px};

use super::{Palette, TextSpec, alpha, icon_colored};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;

pub const HEIGHT: f32 = 61.0;

#[derive(IntoElement)]
pub struct PaneHeader {
    p: Palette,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl PaneHeader {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }
}

impl ParentElement for PaneHeader {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for PaneHeader {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for PaneHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // `flex h-[61px] shrink-0 items-center gap-3 border-b px-4`
        let mut header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.0))
            .h(px(HEIGHT))
            .px(px(16.0))
            .border_b_1()
            .border_color(self.p.t.border)
            .children(self.children);
        header.style().refine(&self.style);
        header
    }
}

/// `ScreenHeader`。アイコンの枠、タイトル、説明、右側の操作を並べる。
#[derive(IntoElement)]
pub struct ScreenHeader {
    p: Palette,
    icon: Icon,
    title: SharedString,
    description: Option<SharedString>,
    actions: Vec<AnyElement>,
}

impl ScreenHeader {
    pub fn new(p: &Palette, icon: Icon, title: impl Into<SharedString>) -> Self {
        Self {
            p: *p,
            icon,
            title: title.into(),
            description: None,
            actions: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for ScreenHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let sm = TextSpec::SM;
        let xs = TextSpec::XS;
        PaneHeader::new(&self.p)
            // `bg-background/80`。`backdrop-blur` は GPUI に無い。
            .bg(alpha(t.background, 0.8))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(radius::LG)
                    .bg(t.brand_muted)
                    .child(icon_colored(self.icon, 16.0, t.brand_muted_foreground)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(sm.size())
                            .line_height(sm.line())
                            .font(fonts::sans(600.0))
                            .child(self.title),
                    )
                    .children(self.description.map(|d| {
                        div()
                            .truncate()
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .text_color(t.muted_foreground)
                            .child(d)
                    })),
            )
            .children(self.actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_is_spacing_15_plus_border() {
        assert_eq!(HEIGHT, 15.0 * 4.0 + 1.0);
    }
}
