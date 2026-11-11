//! `dialog.tsx`。背景の幕、パネル、ヘッダー、フッター、閉じるボタン。
//! 幕は親の領域いっぱいに広げる (Web 版の `fixed inset-0` は、ここでは `relative` な親に当たる)。
//! `backdrop-blur-xs` は GPUI に無いので、黒 10% の幕だけを描く。

use gpui::{
    AnyElement, App, ClickEvent, SharedString, StyleRefinement, Window, div, prelude::*, px,
};

use super::button::{Button, ButtonSize, ButtonVariant};
use super::{Palette, TextSpec, alpha, black, ring};
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
    div().px(px(16.0)).children(children)
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
    p: Palette,
    title: SharedString,
    description: Option<SharedString>,
    show_close_button: bool,
    close_hovered: bool,
    on_close: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    style: StyleRefinement,
}

impl Dialog {
    pub fn new(p: &Palette, title: impl Into<SharedString>) -> Self {
        Self {
            p: *p,
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let has_footer = !self.footer.is_empty();
        let mut panel = panel_frame(&self.p, WIDTH)
            .when(!has_footer, |d| d.pb(px(16.0)))
            .child(
                // `DialogHeader`: `flex flex-col gap-2`
                body([div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(title_text(self.title, 16.0))
                    .children(self.description.map(|d| description_text(&self.p, d)))
                    .into_any_element()]),
            )
            .children(self.body.into_iter().map(|c| body([c])))
            .when(has_footer, |d| {
                d.child(footer_frame(&self.p).children(self.footer))
            });
        if self.show_close_button {
            // `absolute top-2 right-2`、ghost の icon-sm。
            let mut close = Button::new("dialog-close", &self.p)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconSm)
                .icon(Icon::X)
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
