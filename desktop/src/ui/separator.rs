//! `separator.tsx`。1px の線。

use gpui::{App, StyleRefinement, Window, div, prelude::*, px};

use super::Palette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(IntoElement)]
pub struct Separator {
    p: Palette,
    orientation: Orientation,
    style: StyleRefinement,
}

impl Separator {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            orientation: Orientation::Horizontal,
            style: StyleRefinement::default(),
        }
    }

    pub fn vertical(mut self) -> Self {
        self.orientation = Orientation::Vertical;
        self
    }
}

impl Styled for Separator {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Separator {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // `shrink-0 bg-border`、横は `h-px w-full`、縦は `w-px self-stretch` (高さは親に合わせて h_full)。
        let mut line = div().flex_none().bg(self.p.t.border);
        line = match self.orientation {
            Orientation::Horizontal => line.h(px(1.0)).w_full(),
            Orientation::Vertical => line.w(px(1.0)).h_full(),
        };
        line.style().refine(&self.style);
        line
    }
}
