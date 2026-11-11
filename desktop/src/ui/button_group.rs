//! `button-group.tsx`。トグルボタンを枠に並べる。

use gpui::{AnyElement, App, ClickEvent, ElementId, StyleRefinement, Window, div, prelude::*, px};

use super::button::{Button, ButtonSize, ButtonVariant};
use super::{ClickHandler, Palette, alpha, shadow_sm};
use crate::icons::Icon;
use crate::tokens::radius;

/// `ButtonGroupItem` の一辺 (`size-10`)。
pub const ITEM_SIZE: f32 = 40.0;

#[derive(IntoElement)]
pub struct ButtonGroup {
    p: Palette,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl ButtonGroup {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }
}

impl ParentElement for ButtonGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for ButtonGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ButtonGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // `inline-flex shrink-0 items-center gap-0.5 rounded-lg border bg-muted/40 p-0.5`
        let mut group = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded(radius::LG)
            .border_1()
            .border_color(self.p.t.border)
            .bg(alpha(self.p.t.muted, 0.4))
            .children(self.children);
        group.style().refine(&self.style);
        group
    }
}

/// ghost ボタンを 40px 角にしたもの。押されている (`data-pressed`) と primary の配色になる。
#[derive(IntoElement)]
pub struct ButtonGroupItem {
    id: ElementId,
    p: Palette,
    icon: Icon,
    pressed: bool,
    hovered: bool,
    on_click: Option<ClickHandler>,
}

impl ButtonGroupItem {
    pub fn new(id: impl Into<ElementId>, p: &Palette, icon: Icon) -> Self {
        Self {
            id: id.into(),
            p: *p,
            icon,
            pressed: false,
            hovered: false,
            on_click: None,
        }
    }

    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
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

impl RenderOnce for ButtonGroupItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let variant = if self.pressed {
            ButtonVariant::Default
        } else {
            ButtonVariant::Ghost
        };
        let mut button = Button::new(self.id, &self.p)
            .variant(variant)
            .size(ButtonSize::Icon)
            .icon(self.icon)
            .square_px(ITEM_SIZE)
            .rounded_px(f32::from(radius::MD))
            .hovered(self.hovered && !self.pressed);
        if self.pressed {
            button = button.shadow(shadow_sm());
        }
        if let Some(h) = self.on_click {
            button = button.on_click(h);
        }
        button
    }
}
