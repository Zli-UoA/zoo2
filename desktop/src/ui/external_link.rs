//! `external-link.tsx`。リンク色の文字で、押すと外部のブラウザで開く。

use gpui::{App, ElementId, SharedString, StyleRefinement, Window, div, prelude::*};

use super::{alpha, palette, ring};
use crate::tokens::radius;

#[derive(IntoElement)]
pub struct ExternalLink {
    id: ElementId,
    label: SharedString,
    url: SharedString,
    focused: bool,
    style: StyleRefinement,
}

impl ExternalLink {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            url: url.into(),
            focused: false,
            style: StyleRefinement::default(),
        }
    }

    /// `focus-visible:ring-2 ring-ring/50` を常に出す。
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

impl Styled for ExternalLink {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ExternalLink {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let url = self.url.clone();
        // Tailwind の preflight は `a` の下線を消す。`underline-offset-2` は下線を引くときだけ効く。
        let mut link = div()
            .id(self.id)
            .rounded(radius::SM)
            .text_color(p.t.link)
            .cursor_pointer()
            .when(self.focused, |d| {
                d.shadow(vec![ring(alpha(p.t.ring, 0.5), 2.0)])
            })
            .on_click(move |_, _, cx| cx.open_url(&url))
            .child(self.label);
        link.style().refine(&self.style);
        link
    }
}
