//! `tooltip.tsx`。`bg-foreground text-background` の吹き出し。

use gpui::{
    AnyView, App, Bounds, Context, IntoElement, PathBuilder, Pixels, Render, SharedString, Window,
    canvas, div, point, prelude::*, px,
};

use super::{Palette, TextSpec};
use crate::fonts;
use crate::tokens::radius;

/// 矢印の向き。吹き出しから見て、矢印のある側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// 吹き出しは対象の上にあり、矢印は下に出る (既定)。
    Top,
    /// 吹き出しは対象の下にあり、矢印は上に出る。
    Bottom,
}

/// 矢印の三角形の底辺の半分と高さ。
/// `size-2.5 rotate-45` (一辺 10px) の菱形が、端から 2px 内側を中心にしている。
/// 外に出るのは 7.07 - 2 = 5.07px。
pub const ARROW_HALF: f32 = 5.07;

#[derive(IntoElement)]
pub struct Tooltip {
    p: Palette,
    text: SharedString,
    side: Side,
    arrow: bool,
}

impl Tooltip {
    pub fn new(p: &Palette, text: impl Into<SharedString>) -> Self {
        Self {
            p: *p,
            text: text.into(),
            side: Side::Top,
            arrow: true,
        }
    }

    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    pub fn arrow(mut self, arrow: bool) -> Self {
        self.arrow = arrow;
        self
    }
}

fn arrow_canvas(color: gpui::Rgba, side: Side) -> impl IntoElement {
    let w = ARROW_HALF * 2.0;
    div().relative().h(px(0.0)).w_full().child(
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, _, window, _| {
                let cx = bounds.center().x;
                let (base, apex) = match side {
                    Side::Top => (bounds.top(), bounds.top() + px(ARROW_HALF)),
                    Side::Bottom => (bounds.top(), bounds.top() - px(ARROW_HALF)),
                };
                let mut b = PathBuilder::fill();
                b.move_to(point(cx - px(w / 2.0), base));
                b.line_to(point(cx + px(w / 2.0), base));
                b.line_to(point(cx, apex));
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(px(0.0)),
    )
}

impl RenderOnce for Tooltip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let xs = TextSpec::XS;
        // `inline-flex w-fit max-w-xs items-center gap-1.5 rounded-md bg-foreground px-3 py-1.5 text-xs text-background`
        let bubble = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .max_w(px(320.0))
            .px(px(12.0))
            .py(px(6.0))
            .rounded(radius::MD)
            .bg(t.foreground)
            .text_color(t.background)
            .text_size(xs.size())
            .line_height(xs.line())
            .font(fonts::sans(400.0))
            .child(self.text);
        let arrow = self.arrow.then(|| arrow_canvas(t.foreground, self.side));
        let mut col = div().flex().flex_col().items_start();
        match self.side {
            Side::Top => col = col.child(bubble).children(arrow),
            Side::Bottom => col = col.children(arrow).child(bubble),
        }
        col
    }
}

/// 実際のホバーで出すツールチップ。`div().tooltip(tooltip_view(p, "..."))` のように使う。
pub fn tooltip_view(
    p: Palette,
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| TooltipView { p, text }).into()
    }
}

struct TooltipView {
    p: Palette,
    text: SharedString,
}

impl Render for TooltipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // 矢印は付けず、対象との間は GPUI が空ける。
        Tooltip::new(&self.p, self.text.clone()).arrow(false)
    }
}
