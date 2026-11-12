//! `spinner.tsx`。`size-7` の LoaderCircle が `animate-spin` で回る。

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, Context, ElementId, Render, StyleRefinement, Transformation,
    Window, div, percentage, prelude::*,
};

use super::{icon_colored, keyed_entity, palette};
use crate::icons::Icon;

pub const SIZE: f32 = 28.0;

#[derive(IntoElement)]
pub struct Spinner {
    id: ElementId,
    animate: bool,
    style: StyleRefinement,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
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

/// 回る部分だけを持つビュー。毎フレームの通知は、親ではなくこのビューにだけ届く。
struct SpinnerView {
    style: StyleRefinement,
}

impl Render for SpinnerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let glyph = icon_colored(Icon::LoaderCircle, SIZE, p.t.muted_foreground).with_animation(
            "spin",
            Animation::new(Duration::from_secs(1)).repeat(),
            |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
        );
        let mut root = div().flex().items_center().justify_center().child(glyph);
        root.style().refine(&self.style);
        root
    }
}

impl RenderOnce for Spinner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.animate {
            let style = self.style.clone();
            let view = keyed_entity(self.id, window, cx, || SpinnerView {
                style: style.clone(),
            });
            view.update(cx, |v, _| v.style = style);
            return view.into_any_element();
        }
        let p = palette(cx);
        let mut root = div()
            .flex()
            .items_center()
            .justify_center()
            .child(icon_colored(Icon::LoaderCircle, SIZE, p.t.muted_foreground));
        root.style().refine(&self.style);
        root.into_any_element()
    }
}
