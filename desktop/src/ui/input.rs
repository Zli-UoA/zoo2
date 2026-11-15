//! `input.tsx` と `input-styles.ts`。見た目だけの枠で、入力処理 (IME など) は持たない。
//! 既存の `input.rs` (テキスト入力) を中に入れるときは、`child` に渡す。

use gpui::{AnyElement, App, SharedString, StyleRefinement, Window, div, prelude::*, px};

use super::{Palette, TextSpec, alpha, palette, ring};
use crate::fonts;
use crate::tokens::radius;

/// 入力欄の状態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FieldState {
    pub focused: bool,
    pub disabled: bool,
}

/// `inputClassName()`: `h-8 rounded-lg border border-border bg-background px-2 text-sm`
/// と `focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50`。
pub struct InputFrame {
    pub height: f32,
    pub padding_x: f32,
    pub text: TextSpec,
}

pub const INPUT_FRAME: InputFrame = InputFrame {
    height: 32.0,
    padding_x: 8.0,
    text: TextSpec::SM,
};

/// Chromium の無効な `<input>` / `<select>` の文字色 (`-internal-light-dark`)。
pub fn disabled_text(p: &Palette) -> gpui::Rgba {
    p.pick(crate::tokens::hex(0x545454), crate::tokens::hex(0xaaaaaa))
}

/// 入力欄の中の 1 行の文字。長いときは右の余白に食い込まず、省略記号で切る。
fn value_cell() -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
}

/// 入力欄の枠 (子は呼び出し側で足す)。`Input` と `Select` が共有する。
pub fn input_frame(p: &Palette, state: FieldState) -> gpui::Div {
    let t = p.t;
    let f = &INPUT_FRAME;
    div()
        .flex()
        .items_center()
        .h(px(f.height))
        .px(px(f.padding_x))
        .rounded(radius::LG)
        .border_1()
        .border_color(if state.focused { t.ring } else { t.border })
        .bg(t.background)
        .text_size(f.text.size())
        .line_height(f.text.line())
        .font(fonts::sans(400.0))
        .text_color(if state.disabled {
            disabled_text(p)
        } else {
            t.foreground
        })
        .overflow_hidden()
        .when(state.focused, |d| {
            d.shadow(vec![ring(alpha(t.ring, 0.5), 3.0)])
        })
}

#[derive(IntoElement)]
pub struct Input {
    value: Option<SharedString>,
    placeholder: Option<SharedString>,
    state: FieldState,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl Input {
    pub fn new() -> Self {
        Self {
            value: None,
            placeholder: None,
            state: FieldState::default(),
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

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    /// `focus-visible` の見た目 (枠の色と輪) を出す。呼び出し側が、中の入力欄にフォーカスがあるかを
    /// render で調べて渡す。同じ FocusHandle を枠でも `track_focus` すると、tab stop と
    /// フォーカスの経路が二重になるので、枠はフォーカスを持たない。
    pub fn focused(mut self, focused: bool) -> Self {
        self.state.focused = focused;
        self
    }
}

impl ParentElement for Input {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Input {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let shown = match (&self.value, &self.placeholder) {
            (Some(v), _) if !v.is_empty() => Some(value_cell().child(v.clone())),
            // Tailwind v4 の placeholder は `currentcolor` の 50%。
            (_, Some(ph)) => Some(
                value_cell()
                    .text_color(alpha(t.foreground, 0.5))
                    .child(ph.clone()),
            ),
            _ => None,
        };
        let mut frame = input_frame(&p, self.state)
            .children(shown)
            .children(self.children);
        frame.style().refine(&self.style);
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_matches_input_class() {
        // h-8 px-2 rounded-lg text-sm
        assert_eq!(INPUT_FRAME.height, 32.0);
        assert_eq!(INPUT_FRAME.padding_x, 8.0);
        assert_eq!(INPUT_FRAME.text, TextSpec::SM);
    }

    #[test]
    fn disabled_text_follows_mode() {
        use crate::tokens::ThemeMode;
        let l = disabled_text(&Palette::new(ThemeMode::Light));
        let d = disabled_text(&Palette::new(ThemeMode::Dark));
        assert!(l.r < d.r);
    }
}
