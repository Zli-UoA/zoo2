//! `skeleton.tsx`。`animate-pulse rounded-md bg-muted`。

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, Context, ElementId, Render, StyleRefinement, Window, div,
    prelude::*,
};

use super::{Palette, keyed_entity, palette};
use crate::tokens::radius;

/// `animate-pulse` の不透明度。0% と 100% が 1、50% が 0.5。
pub fn pulse_opacity(delta: f32) -> f32 {
    1.0 - 0.5 * (1.0 - (delta * std::f32::consts::TAU).cos()) / 2.0
}

#[derive(IntoElement)]
pub struct Skeleton {
    id: ElementId,
    animate: bool,
    style: StyleRefinement,
}

impl Skeleton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            animate: true,
            style: StyleRefinement::default(),
        }
    }

    /// false なら静止して描く (撮影用)。
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }
}

impl Styled for Skeleton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// 脈打つ部分だけを持つビュー。毎フレームの通知は、親ではなくこのビューにだけ届く。
struct SkeletonView {
    style: StyleRefinement,
}

fn bar(p: &Palette, style: &StyleRefinement) -> gpui::Div {
    let mut bar = div().rounded(radius::MD).bg(p.t.muted);
    bar.style().refine(style);
    bar
}

impl Render for SkeletonView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        bar(&p, &self.style).with_animation(
            "pulse",
            Animation::new(Duration::from_secs(2)).repeat(),
            |bar, delta| bar.opacity(pulse_opacity(delta)),
        )
    }
}

impl RenderOnce for Skeleton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.animate {
            let style = self.style.clone();
            let view = keyed_entity(self.id, window, cx, || SkeletonView {
                style: style.clone(),
            });
            view.update(cx, |v, _| v.style = style);
            return view.into_any_element();
        }
        bar(&palette(cx), &self.style).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_goes_down_to_half_and_back() {
        assert!((pulse_opacity(0.0) - 1.0).abs() < 1e-6);
        assert!((pulse_opacity(0.5) - 0.5).abs() < 1e-6);
        assert!((pulse_opacity(1.0) - 1.0).abs() < 1e-5);
    }
}
