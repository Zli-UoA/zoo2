//! UI 部品の確認画面 (`zoo2-desktop gallery components [ページ]`)。
//! Web 版の `*.stories.tsx` と同じ状態を並べ、見比べられるようにする。

use gpui::{
    AnyElement, Context, Div, IntoElement, Render, SharedString, Window, div, prelude::*, px,
};

use crate::fonts;
use crate::tokens::{ThemeMode, font_size, radius};
use crate::ui::Palette;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};

/// ページ数。
pub const PAGES: usize = 1;

pub struct ComponentsGallery {
    page: usize,
    p: Palette,
}

impl ComponentsGallery {
    pub fn new(page: usize, mode: ThemeMode) -> Self {
        Self {
            page: page.clamp(1, PAGES),
            p: Palette::new(mode),
        }
    }
}

/// 部品の名前を付けた枠。
fn section(p: &Palette, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(12.0))
        .border_1()
        .border_color(p.t.border)
        .rounded(radius::LG)
        .child(
            div()
                .text_size(font_size::XS)
                .font(fonts::sans(600.0))
                .text_color(p.t.muted_foreground)
                .child(title.into()),
        )
}

fn row() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.0))
}

fn buttons(p: &Palette) -> AnyElement {
    let mut variants = row();
    for v in ButtonVariant::ALL {
        variants = variants.child(
            Button::new(SharedString::from(format!("v-{}", v.name())), p)
                .variant(v)
                .label(v.name()),
        );
    }
    let mut hovered = row();
    for v in ButtonVariant::ALL {
        hovered = hovered.child(
            Button::new(SharedString::from(format!("h-{}", v.name())), p)
                .variant(v)
                .hovered(true)
                .label(v.name()),
        );
    }
    let mut disabled = row();
    for v in ButtonVariant::ALL {
        disabled = disabled.child(
            Button::new(SharedString::from(format!("d-{}", v.name())), p)
                .variant(v)
                .disabled(true)
                .label(v.name()),
        );
    }
    let mut sizes = row();
    for s in ButtonSize::ALL {
        let b = Button::new(SharedString::from(format!("s-{}", s.name())), p).size(s);
        sizes = sizes.child(match s {
            ButtonSize::Icon | ButtonSize::IconXs | ButtonSize::IconSm | ButtonSize::IconLg => {
                b.icon(crate::icons::Icon::Plus)
            }
            _ => b.label(s.name()),
        });
    }
    section(p, "Button")
        .child(variants)
        .child(hovered)
        .child(disabled)
        .child(sizes)
        .into_any_element()
}

impl Render for ComponentsGallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.p;
        let body = match self.page {
            _ => div().flex().flex_col().gap(px(12.0)).child(buttons(&p)),
        };
        div()
            .size_full()
            .p(px(16.0))
            .bg(p.t.background)
            .text_color(p.t.foreground)
            .font(fonts::sans(400.0))
            .text_size(font_size::SM)
            .child(body)
    }
}
