//! `skeleton.tsx`。`animate-pulse rounded-md bg-muted`。

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, ElementId, StyleRefinement, Window, div, prelude::*,
};

use super::Palette;
use crate::tokens::radius;

/// `animate-pulse` の不透明度。0% と 100% が 1、50% が 0.5。
pub fn pulse_opacity(delta: f32) -> f32 {
    1.0 - 0.5 * (1.0 - (delta * std::f32::consts::TAU).cos()) / 2.0
}

#[derive(IntoElement)]
pub struct Skeleton {
    id: ElementId,
    p: Palette,
    animate: bool,
    style: StyleRefinement,
}

impl Skeleton {
    pub fn new(id: impl Into<ElementId>, p: &Palette) -> Self {
        Self {
            id: id.into(),
            p: *p,
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

impl RenderOnce for Skeleton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut bar = div().rounded(radius::MD).bg(self.p.t.muted);
        bar.style().refine(&self.style);
        if self.animate {
            bar.with_animation(
                self.id,
                Animation::new(Duration::from_secs(2)).repeat(),
                |bar, delta| bar.opacity(pulse_opacity(delta)),
            )
            .into_any_element()
        } else {
            bar.into_any_element()
        }
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
