//! `textarea.tsx`。見た目だけの枠。高さは内容に合わせて伸び、最小は `min-h-16` (64px)。

use gpui::{
    AnyElement, App, FocusHandle, SharedString, StyleRefinement, Window, div, prelude::*, px,
};

use super::{Palette, TextSpec, alpha, palette, ring};
use crate::fonts;
use crate::tokens::radius;

pub const MIN_HEIGHT: f32 = 64.0;
/// `text-base md:text-sm`。デスクトップは md 以上なので 14px。
pub const TEXT: TextSpec = TextSpec::SM;

/// 枠の色と輪。(枠の色、背景、輪、不透明度)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextareaColors {
    pub border: gpui::Rgba,
    pub bg: gpui::Rgba,
    pub ring: Option<(gpui::Rgba, f32)>,
    pub opacity: f32,
}

pub fn colors(p: &Palette, focused: bool, disabled: bool, invalid: bool) -> TextareaColors {
    let t = p.t;
    let mut border = t.input;
    // `bg-transparent dark:bg-input/30`、無効なら `bg-input/50 dark:bg-input/80`。
    let mut bg = p.pick(super::TRANSPARENT, alpha(t.input, 0.3));
    let mut ring_style = None;
    if focused {
        border = t.ring;
        ring_style = Some((alpha(t.ring, 0.5), 3.0));
    }
    if invalid {
        border = p.pick(t.destructive, alpha(t.destructive, 0.5));
        ring_style = Some((alpha(t.destructive, p.pick(0.2, 0.4)), 3.0));
    }
    if disabled {
        bg = alpha(t.input, p.pick(0.5, 0.8));
    }
    TextareaColors {
        border,
        bg,
        ring: ring_style,
        opacity: if disabled { 0.5 } else { 1.0 },
    }
}

/// 実フォーカス時 (`.focus(...)`) に重ねる枠と輪。`colors()` の focused 版をそのまま使うので、
/// invalid は focus より優先される。
pub fn focus_style(p: &Palette, disabled: bool, invalid: bool) -> TextareaColors {
    colors(p, true, disabled, invalid)
}

#[derive(IntoElement)]
pub struct Textarea {
    value: Option<SharedString>,
    placeholder: Option<SharedString>,
    focused: bool,
    disabled: bool,
    invalid: bool,
    focus_handle: Option<FocusHandle>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl Textarea {
    pub fn new() -> Self {
        Self {
            value: None,
            placeholder: None,
            focused: false,
            disabled: false,
            invalid: false,
            focus_handle: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `aria-invalid`。
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn focus_handle(mut self, handle: &FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }
}

impl ParentElement for Textarea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Textarea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Textarea {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let c = colors(&p, self.focused, self.disabled, self.invalid);
        let shown = match (&self.value, &self.placeholder) {
            (Some(v), _) if !v.is_empty() => Some(div().child(v.clone())),
            (_, Some(ph)) => Some(div().text_color(t.muted_foreground).child(ph.clone())),
            _ => None,
        };
        // `flex min-h-16 w-full rounded-lg border border-input px-2.5 py-2`
        let mut area = div()
            .flex()
            .flex_col()
            .w_full()
            .min_h(px(MIN_HEIGHT))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(radius::LG)
            .border_1()
            .border_color(c.border)
            .bg(c.bg)
            .text_size(TEXT.size())
            .line_height(TEXT.line())
            .font(fonts::sans(400.0))
            .text_color(t.foreground)
            .opacity(c.opacity)
            .when_some(c.ring, |d, (color, w)| d.shadow(vec![ring(color, w)]))
            .children(shown)
            .children(self.children);
        if let Some(handle) = self.focus_handle.as_ref() {
            let focus = focus_style(&p, self.disabled, self.invalid);
            area = area.track_focus(handle).focus(move |s| {
                let s = s.border_color(focus.border);
                match focus.ring {
                    Some((color, w)) => s.shadow(vec![ring(color, w)]),
                    None => s,
                }
            });
        }
        area.style().refine(&self.style);
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::ThemeMode;

    #[test]
    fn disabled_and_invalid_colors() {
        let p = Palette::new(ThemeMode::Light);
        let d = colors(&p, false, true, false);
        assert_eq!(d.opacity, 0.5);
        assert!((d.bg.a - 0.5).abs() < 1e-6);
        let i = colors(&p, false, false, true);
        assert_eq!(i.border, p.t.destructive);
        assert_eq!(i.ring.map(|r| r.1), Some(3.0));
    }

    #[test]
    fn dark_background_is_input_30() {
        let p = Palette::new(ThemeMode::Dark);
        let c = colors(&p, false, false, false);
        assert!((c.bg.a - 0.3).abs() < 1e-6);
        let d = colors(&p, false, true, false);
        assert!((d.bg.a - 0.8).abs() < 1e-6);
    }

    #[test]
    fn real_focus_style_keeps_invalid() {
        let p = Palette::new(ThemeMode::Light);
        let f = focus_style(&p, false, true);
        assert_eq!(f.border, p.t.destructive);
        assert_eq!(f.ring.map(|r| r.0), colors(&p, false, false, true).ring.map(|r| r.0));
        let plain = focus_style(&p, false, false);
        assert_eq!(plain.border, p.t.ring);
    }

    #[test]
    fn invalid_beats_focus_for_the_ring() {
        let p = Palette::new(ThemeMode::Light);
        let c = colors(&p, true, false, true);
        assert_eq!(c.border, p.t.destructive);
    }
}
