//! `select.tsx`。Web 版はネイティブの `<select>` で、枠は `inputClassName()` に `w-40` を足したもの。
//! ここでは閉じた枠と、開いたときの一覧 (メニューと同じ見た目) を静止画で描く。

use gpui::{AnyElement, App, SharedString, StyleRefinement, Window, div, prelude::*, px};

use super::dropdown_menu::{MenuItem, popup_frame};
use super::input::{FieldState, disabled_text, input_frame};
use super::{Palette, icon_colored};
use crate::icons::Icon;

/// `w-40`
pub const WIDTH: f32 = 160.0;

#[derive(IntoElement)]
pub struct Select {
    p: Palette,
    value: SharedString,
    state: FieldState,
    style: StyleRefinement,
}

impl Select {
    pub fn new(p: &Palette, value: impl Into<SharedString>) -> Self {
        Self {
            p: *p,
            value: value.into(),
            state: FieldState::default(),
            style: StyleRefinement::default(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.state.focused = focused;
        self
    }
}

impl Styled for Select {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Select {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let arrow = if self.state.disabled {
            disabled_text(&self.p)
        } else {
            t.foreground
        };
        // `w-40 max-w-full disabled:cursor-not-allowed disabled:opacity-60`
        let mut select = input_frame(&self.p, self.state)
            .w(px(WIDTH))
            .max_w_full()
            .justify_between()
            .gap(px(4.0))
            .opacity(if self.state.disabled { 0.6 } else { 1.0 })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .child(self.value),
            )
            .child(icon_colored(Icon::ChevronDown, 12.0, arrow));
        select.style().refine(&self.style);
        select
    }
}

/// 開いた一覧。選ばれている項目には Check が付く。
#[derive(IntoElement)]
pub struct SelectPopup {
    p: Palette,
    options: Vec<(SharedString, bool)>,
    highlighted: Option<usize>,
}

impl SelectPopup {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            options: Vec::new(),
            highlighted: None,
        }
    }

    pub fn option(mut self, label: impl Into<SharedString>, selected: bool) -> Self {
        self.options.push((label.into(), selected));
        self
    }

    pub fn highlighted(mut self, index: usize) -> Self {
        self.highlighted = Some(index);
        self
    }
}

impl RenderOnce for SelectPopup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let items: Vec<AnyElement> = self
            .options
            .into_iter()
            .enumerate()
            .map(|(i, (label, selected))| {
                MenuItem::new(
                    SharedString::from(format!("select-option-{i}")),
                    &self.p,
                    label,
                )
                .indicator(super::dropdown_menu::Indicator::Radio(selected))
                .highlighted(self.highlighted == Some(i))
                .into_any_element()
            })
            .collect();
        popup_frame(&self.p).w(px(WIDTH)).children(items)
    }
}
