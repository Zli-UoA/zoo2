//! `dialog.tsx`。背景の幕、パネル、ヘッダー、フッター、閉じるボタン。
//! 幕は親の領域いっぱいに広げる (Web 版の `fixed inset-0` は、ここでは `relative` な親に当たる)。
//! `backdrop-blur-xs` は GPUI に無いので、黒 10% の幕だけを描く。

use gpui::{
    AnyElement, App, ClickEvent, MouseButton, SharedString, StyleRefinement, Window, div, prelude::*, px,
};

use super::button::{Button, ButtonSize, ButtonVariant};
use super::{ClickHandler, Palette, TextSpec, alpha, black, palette, ring, v_flex};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;

/// `sm:max-w-sm`
pub const WIDTH: f32 = 384.0;

/// `fixed inset-0 bg-black/10` の幕に、パネルを中央寄せで載せたもの。
pub fn overlay(panel: impl IntoElement) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(black(0.1))
        // 幕の外側のマウス操作を背後に通さない。
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(panel)
}

/// パネルの箱。`rounded-xl bg-popover text-sm text-popover-foreground ring-1 ring-foreground/10`
pub fn panel_frame(p: &Palette, width: f32) -> gpui::Div {
    let t = p.t;
    let sm = TextSpec::SM;
    div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .w(px(width))
        .max_w_full()
        .pt(px(16.0))
        .rounded(radius::XL)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .text_size(sm.size())
        .line_height(sm.line())
        .font(fonts::sans(400.0))
        .shadow(vec![ring(alpha(t.foreground, 0.1), 1.0)])
}

/// 本文の左右の余白 (`p-4`)。
pub fn body(children: impl IntoIterator<Item = AnyElement>) -> gpui::Div {
    v_flex().px(px(16.0)).children(children)
}

/// `DialogFooter`: `-mx-4 -mb-4 flex gap-2 rounded-b-xl border-t bg-muted/50 p-4 justify-end`
pub fn footer_frame(p: &Palette) -> gpui::Div {
    div()
        .flex()
        .gap(px(8.0))
        .justify_end()
        .p(px(16.0))
        .rounded_b(radius::XL)
        .border_t_1()
        .border_color(p.t.border)
        .bg(alpha(p.t.muted, 0.5))
}

/// `DialogTitle`: `text-base leading-none font-medium`
pub fn title_text(text: impl Into<SharedString>, line: f32) -> gpui::Div {
    div()
        .text_size(TextSpec::BASE.size())
        .line_height(px(line))
        .font(fonts::sans(500.0))
        .child(text.into())
}

/// `DialogDescription`: `text-sm text-muted-foreground`
pub fn description_text(p: &Palette, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(TextSpec::SM.size())
        .line_height(TextSpec::SM.line())
        .text_color(p.t.muted_foreground)
        .child(text.into())
}

#[derive(IntoElement)]
pub struct Dialog {
    id: SharedString,
    title: SharedString,
    description: Option<SharedString>,
    show_close_button: bool,
    close_hovered: bool,
    on_close: Option<ClickHandler>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    style: StyleRefinement,
}

impl Dialog {
    /// `id` は閉じるボタンの ID の元になる。同じ画面に複数あるときは、別の値にする。
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            show_close_button: true,
            close_hovered: false,
            on_close: None,
            body: Vec::new(),
            footer: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn show_close_button(mut self, show: bool) -> Self {
        self.show_close_button = show;
        self
    }

    pub fn close_hovered(mut self, hovered: bool) -> Self {
        self.close_hovered = hovered;
        self
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(handler));
        self
    }

    /// フッターに並べる (ボタンなど)。
    pub fn footer(mut self, child: impl IntoElement) -> Self {
        self.footer.push(child.into_any_element());
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl Styled for Dialog {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Dialog {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let has_footer = !self.footer.is_empty();
        let mut panel = panel_frame(&p, WIDTH)
            .when(!has_footer, |d| d.pb(px(16.0)))
            .child(
                // `DialogHeader`: `flex flex-col gap-2`
                body([div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(title_text(self.title, 16.0))
                    .children(self.description.map(|d| description_text(&p, d)))
                    .into_any_element()]),
            )
            .children(self.body.into_iter().map(|c| body([c])))
            .when(has_footer, |d| {
                d.child(footer_frame(&p).children(self.footer))
            });
        if self.show_close_button {
            // `absolute top-2 right-2`、ghost の icon-sm。
            let mut close = Button::new(SharedString::from(format!("{}-close", self.id)))
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconSm)
                .icon_start(Icon::X)
                .hovered(self.close_hovered)
                .absolute()
                .top(px(8.0))
                .right(px(8.0));
            if let Some(h) = self.on_close {
                close = close.on_click(h);
            }
            panel = panel.child(close);
        }
        panel.style().refine(&self.style);
        panel
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{Context, TestAppContext, point, px};

    use super::*;

    /// 背後に 100x100 のボタン、その上に (幕あり/なしで) 全面の層と、右下に 40x40 のパネル内ボタン。
    struct Root {
        behind: Rc<Cell<u32>>,
        inside: Rc<Cell<u32>>,
        with_overlay: bool,
    }

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let behind = self.behind.clone();
            let inside = self.inside.clone();
            let button = div()
                .id("behind")
                .absolute()
                .top_0()
                .left_0()
                .size(px(100.0))
                .on_click(move |_, _, _| behind.set(behind.get() + 1));
            let panel = div().id("panel-button").size(px(40.0)).on_click(move |_, _, _| {
                inside.set(inside.get() + 1)
            });
            let root = div().relative().size_full().child(button);
            if self.with_overlay {
                root.child(overlay(panel))
            } else {
                root
            }
        }
    }

    fn click(with_overlay: bool, at: gpui::Point<gpui::Pixels>) -> (u32, u32) {
        let behind = Rc::new(Cell::new(0));
        let inside = Rc::new(Cell::new(0));
        let (b, i) = (behind.clone(), inside.clone());
        let cx = &mut TestAppContext::single();
        let (_, cx) = cx.add_window_view(move |_, _| Root {
            behind: b,
            inside: i,
            with_overlay,
        });
        cx.simulate_click(at, gpui::Modifiers::none());
        (behind.get(), inside.get())
    }

    #[test]
    fn overlay_blocks_clicks_to_what_is_behind() {
        // パネル (中央の 40x40) の外、背後のボタン (左上 100x100) の上。
        let outside = point(px(10.0), px(10.0));
        assert_eq!(click(false, outside).0, 1, "幕なしの対照では背後に届く");
        assert_eq!(click(true, outside).0, 0, "幕があれば背後に届かない");
    }

    #[test]
    fn panel_content_stays_clickable_under_overlay() {
        // ウィンドウ中央 (既定 1024x768 ではなく、実際の大きさから求める)。
        let behind = Rc::new(Cell::new(0));
        let inside = Rc::new(Cell::new(0));
        let (b, i) = (behind.clone(), inside.clone());
        let cx = &mut TestAppContext::single();
        let (_, cx) = cx.add_window_view(move |_, _| Root {
            behind: b,
            inside: i,
            with_overlay: true,
        });
        let size = cx.update(|window, _| window.viewport_size());
        let center = point(size.width / 2.0, size.height / 2.0);
        cx.simulate_click(center, gpui::Modifiers::none());
        assert_eq!(inside.get(), 1);
        assert_eq!(behind.get(), 0);
    }
}
