//! `spinner.tsx`。`size-7` の LoaderCircle が `animate-spin` で回る。

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, ElementId, StyleRefinement, Transformation, Window, div,
    percentage, prelude::*,
};

use super::{Palette, icon_colored};
use crate::icons::Icon;

pub const SIZE: f32 = 28.0;

#[derive(IntoElement)]
pub struct Spinner {
    id: ElementId,
    p: Palette,
    animate: bool,
    style: StyleRefinement,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>, p: &Palette) -> Self {
        Self {
            id: id.into(),
            p: *p,
            animate: true,
            style: StyleRefinement::default(),
        }
    }

    /// false なら回さずに描く (撮影用、`motion-reduce` 相当)。
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }
}

impl Styled for Spinner {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Spinner {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let glyph = icon_colored(Icon::LoaderCircle, SIZE, self.p.t.muted_foreground);
        let glyph = if self.animate {
            glyph
                .with_animation(
                    self.id,
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
        } else {
            glyph.into_any_element()
        };
        let mut root = div().flex().items_center().justify_center().child(glyph);
        root.style().refine(&self.style);
        root
    }
}
